package com.waghitech.krapy

import java.net.URI
import java.net.URLDecoder
import java.nio.charset.StandardCharsets
import java.util.Base64

class PairingException(message: String) : Exception(message)

data class PairingData(
    val host: String,
    val port: Int,
    val desktopId: String,
    val desktopName: String,
    val secret: ByteArray,
)

object Pairing {
    private val idHex = Regex("^[0-9a-fA-F]{16}$")

    fun parse(uriString: String): PairingData {
        val trimmed = uriString.trim()
        if (trimmed.isEmpty()) throw PairingException("empty uri")
        val uri = try {
            URI(trimmed)
        } catch (e: Exception) {
            throw PairingException("invalid uri: ${e.message}")
        }
        if (uri.scheme != "krapy") throw PairingException("scheme must be krapy")
        if (uri.host != "pair") throw PairingException("host must be pair")
        val q = uri.rawQuery ?: throw PairingException("missing query")
        val params = linkedMapOf<String, String>()
        for (part in q.split("&")) {
            if (part.isEmpty()) continue
            val eq = part.indexOf('=')
            if (eq < 0) throw PairingException("bad query segment")
            val key = URLDecoder.decode(part.substring(0, eq), StandardCharsets.UTF_8)
            val value = URLDecoder.decode(part.substring(eq + 1), StandardCharsets.UTF_8)
            params[key] = value
        }
        if (params["v"] != "1") throw PairingException("v must be 1")
        val host = params["h"]?.takeIf { it.isNotEmpty() }
            ?: throw PairingException("missing h")
        val port = params["p"]?.toIntOrNull()?.takeIf { it in 1..65535 }
            ?: throw PairingException("missing or invalid p")
        val kEnc = params["k"]?.takeIf { it.isNotEmpty() }
            ?: throw PairingException("missing k")
        val desktopId = params["id"]?.takeIf { idHex.matches(it) }
            ?: throw PairingException("id must be 16 hex chars")
        val nameEnc = params["n"] ?: ""
        val desktopName = URLDecoder.decode(nameEnc, StandardCharsets.UTF_8)
        if (desktopName.isEmpty()) throw PairingException("missing n")
        val k = decodeBase64Url(kEnc)
        if (k.size != 32) throw PairingException("k must decode to 32 bytes")
        val salt = "krapy-v1".toByteArray(StandardCharsets.UTF_8)
        val info = "pair".toByteArray(StandardCharsets.UTF_8)
        val secret = Hkdf.sha256(k, salt, info, 32)
        return PairingData(host, port, desktopId.lowercase(), desktopName, secret)
    }

    fun decodeBase64Url(input: String): ByteArray {
        var s = input.trim().replace('-', '+').replace('_', '/')
        val pad = (4 - s.length % 4) % 4
        s += "=".repeat(pad)
        return try {
            Base64.getDecoder().decode(s)
        } catch (e: Exception) {
            throw PairingException("invalid base64url for k")
        }
    }
}