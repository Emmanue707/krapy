package com.waghitech.krapy

import java.nio.ByteBuffer
import java.nio.ByteOrder
import javax.crypto.Cipher
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

private const val REPLAY_WINDOW = 1024L

enum class CryptoError {
    Tamper,
    Replay,
    OldCounter,
}

data class SessionKeys(
    val tcpP2d: ByteArray,
    val tcpD2p: ByteArray,
    val udpP2d: ByteArray,
    val udpD2p: ByteArray,
) {
    companion object {
        fun derive(s: ByteArray, nonceP: ByteArray, nonceD: ByteArray): SessionKeys {
            val salt = ByteArray(32)
            System.arraycopy(nonceP, 0, salt, 0, 16)
            System.arraycopy(nonceD, 0, salt, 16, 16)
            val info = "krapy session".toByteArray(Charsets.UTF_8)
            val okm = Hkdf.sha256(s, salt, info, 128)
            return SessionKeys(
                tcpP2d = okm.copyOfRange(0, 32),
                tcpD2p = okm.copyOfRange(32, 64),
                udpP2d = okm.copyOfRange(64, 96),
                udpD2p = okm.copyOfRange(96, 128),
            )
        }
    }
}

class Sealer(key: ByteArray) {
    private val secret = SecretKeySpec(key, "ChaCha20")
    private var counter: Long = 0

    fun counter(): Long = counter

    private fun nonce(): ByteArray {
        val n = ByteArray(12)
        ByteBuffer.wrap(n).order(ByteOrder.BIG_ENDIAN).putInt(0).putLong(counter)
        return n
    }

    fun sealTcp(plaintext: ByteArray): ByteArray {
        val counterBytes = ByteBuffer.allocate(8).order(ByteOrder.BIG_ENDIAN).putLong(counter).array()
        val cipher = Cipher.getInstance("ChaCha20-Poly1305")
        cipher.init(Cipher.ENCRYPT_MODE, secret, IvParameterSpec(nonce()))
        cipher.updateAAD(counterBytes)
        val ctTag = cipher.doFinal(plaintext)
        counter++
        val out = ByteArray(8 + ctTag.size)
        System.arraycopy(counterBytes, 0, out, 0, 8)
        System.arraycopy(ctTag, 0, out, 8, ctTag.size)
        return out
    }

    fun sealUdp(header9: ByteArray, plaintext: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("ChaCha20-Poly1305")
        cipher.init(Cipher.ENCRYPT_MODE, secret, IvParameterSpec(nonce()))
        cipher.updateAAD(header9)
        val ctTag = cipher.doFinal(plaintext)
        counter++
        val out = ByteArray(9 + ctTag.size)
        System.arraycopy(header9, 0, out, 0, 9)
        System.arraycopy(ctTag, 0, out, 9, ctTag.size)
        return out
    }
}

class Opener(key: ByteArray) {
    private val secret = SecretKeySpec(key, "ChaCha20")
    private var highest: Long? = null
    private var seen: ULong = 0u

    private fun nonce(counter: Long): ByteArray {
        val n = ByteArray(12)
        ByteBuffer.wrap(n).order(ByteOrder.BIG_ENDIAN).putInt(0).putLong(counter)
        return n
    }

    private fun checkReplay(counter: Long): CryptoError? {
        val high = highest
        if (high == null) {
            highest = counter
            seen = 1u
            return null
        }
        if (counter > high) {
            val shift = counter - high
            seen = if (shift >= REPLAY_WINDOW) {
                1u
            } else {
                (seen shl shift.toInt()) or 1u
            }
            highest = counter
            return null
        }
        val behind = high - counter
        if (behind >= REPLAY_WINDOW) return CryptoError.OldCounter
        val bit = 1uL shl behind.toInt()
        if (seen and bit != 0uL) return CryptoError.Replay
        seen = seen or bit
        return null
    }

    fun openTcp(wire: ByteArray): ByteArray {
        if (wire.size < 24) throw CryptoException(CryptoError.Tamper)
        val counter = ByteBuffer.wrap(wire, 0, 8).order(ByteOrder.BIG_ENDIAN).long
        checkReplay(counter)?.let { throw CryptoException(it) }
        val counterBytes = wire.copyOfRange(0, 8)
        val ctTag = wire.copyOfRange(8, wire.size)
        val cipher = Cipher.getInstance("ChaCha20-Poly1305")
        cipher.init(Cipher.DECRYPT_MODE, secret, IvParameterSpec(nonce(counter)))
        cipher.updateAAD(counterBytes)
        return try {
            cipher.doFinal(ctTag)
        } catch (_: Exception) {
            throw CryptoException(CryptoError.Tamper)
        }
    }

    fun openUdp(wire: ByteArray): ByteArray {
        if (wire.size < 25) throw CryptoException(CryptoError.Tamper)
        val counter = ByteBuffer.wrap(wire, 1, 8).order(ByteOrder.BIG_ENDIAN).long
        checkReplay(counter)?.let { throw CryptoException(it) }
        val aad = wire.copyOfRange(0, 9)
        val ctTag = wire.copyOfRange(9, wire.size)
        val cipher = Cipher.getInstance("ChaCha20-Poly1305")
        cipher.init(Cipher.DECRYPT_MODE, secret, IvParameterSpec(nonce(counter)))
        cipher.updateAAD(aad)
        return try {
            cipher.doFinal(ctTag)
        } catch (_: Exception) {
            throw CryptoException(CryptoError.Tamper)
        }
    }
}

class CryptoException(val error: CryptoError) : Exception(error.name)