package com.waghitech.krapy

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test
import java.nio.charset.StandardCharsets

class CryptoTest {
    private fun hex(s: String): ByteArray {
        val out = ByteArray(s.length / 2)
        for (i in out.indices) {
            out[i] = s.substring(i * 2, i * 2 + 2).toInt(16).toByte()
        }
        return out
    }

    @Test
    fun protocolVectors() {
        val k = ByteArray(32) { it.toByte() }
        val salt = "krapy-v1".toByteArray(StandardCharsets.UTF_8)
        val infoPair = "pair".toByteArray(StandardCharsets.UTF_8)
        val s = Hkdf.sha256(k, salt, infoPair, 32)
        assertArrayEquals(
            hex("652e39ae96eb24d8546fe67d80acb68c9710e974f062d7c4486e0be6657fb38b"),
            s,
        )
        val nonceP = ByteArray(16) { 1 }
        val nonceD = ByteArray(16) { 2 }
        val keys = SessionKeys.derive(s, nonceP, nonceD)
        assertArrayEquals(
            hex("c14af774e5c109ed4ef46e6b15c8718ac80b0d50eeb8f754def92a441e6fb3b4"),
            keys.tcpP2d,
        )
        assertArrayEquals(
            hex("f8ce0d3e603791fe2648a067c98a537969b190eb20a9379c20dd3e4d5f06ac7b"),
            keys.udpD2p,
        )
        val sealed = Sealer(keys.tcpP2d).sealTcp("krapy".toByteArray(StandardCharsets.UTF_8))
        assertArrayEquals(
            hex("0000000000000000d483e26a1b70807bc388d46700f5be3dac1fe382e4"),
            sealed,
        )
    }

    @Test
    fun sealOpenRoundtrip() {
        val key = ByteArray(32) { 7 }
        val seal = Sealer(key)
        val open = Opener(key)
        val pt = "hello control".toByteArray(StandardCharsets.UTF_8)
        val out = open.openTcp(seal.sealTcp(pt))
        assertArrayEquals(pt, out)
    }

    @Test(expected = CryptoException::class)
    fun tamperRejected() {
        val key = ByteArray(32) { 1 }
        val seal = Sealer(key)
        val open = Opener(key)
        val wire = seal.sealTcp(byteArrayOf('x'.code.toByte()))
        wire[10] = (wire[10].toInt() xor 0xff).toByte()
        open.openTcp(wire)
    }

    @Test(expected = CryptoException::class)
    fun replayRejected() {
        val key = ByteArray(32) { 2 }
        val seal = Sealer(key)
        val open = Opener(key)
        val wire = seal.sealTcp(byteArrayOf('a'.code.toByte()))
        open.openTcp(wire)
        open.openTcp(wire)
    }

    @Test(expected = CryptoException::class)
    fun oldCounterRejected() {
        val key = ByteArray(32) { 3 }
        val seal = Sealer(key)
        val open = Opener(key)
        var saved: ByteArray? = null
        for (i in 0 until 1040) {
            val wire = seal.sealTcp(byteArrayOf('z'.code.toByte()))
            if (i == 5) {
                saved = wire
                continue
            }
            open.openTcp(wire)
        }
        open.openTcp(saved!!)
    }
}