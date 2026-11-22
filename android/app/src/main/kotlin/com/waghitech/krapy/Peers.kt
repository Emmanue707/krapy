package com.waghitech.krapy

import android.content.Context
import android.os.Build
import org.json.JSONArray
import org.json.JSONObject
import java.security.SecureRandom
import java.util.Base64

data class Peer(
    val id: String,
    val name: String,
    val secret: ByteArray,
    val host: String,
    val port: Int,
    val lastSeen: Long,
) {
    override fun equals(other: Any?): Boolean {
        if (other !is Peer) return false
        return id == other.id && name == other.name && host == other.host &&
            port == other.port && lastSeen == other.lastSeen && secret.contentEquals(other.secret)
    }

    override fun hashCode(): Int = id.hashCode()
}

class Peers(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("krapy_peers", Context.MODE_PRIVATE)
    private val random = SecureRandom()

    fun phoneId(): String {
        var id = prefs.getString("phone_id", null)
        if (id == null) {
            val bytes = ByteArray(8)
            random.nextBytes(bytes)
            id = bytes.joinToString("") { "%02x".format(it) }
            prefs.edit().putString("phone_id", id).apply()
        }
        return id
    }

    fun phoneName(): String = Build.MODEL ?: "android"

    fun list(): List<Peer> {
        val raw = prefs.getString("peers", "[]") ?: "[]"
        val arr = JSONArray(raw)
        val out = ArrayList<Peer>(arr.length())
        for (i in 0 until arr.length()) {
            out.add(peerFromJson(arr.getJSONObject(i)))
        }
        return out
    }

    fun upsert(peer: Peer) {
        val all = list().toMutableList()
        val idx = all.indexOfFirst { it.id == peer.id }
        if (idx >= 0) all[idx] = peer else all.add(peer)
        saveAll(all)
    }

    fun forget(id: String) {
        val all = list().filter { it.id != id }
        saveAll(all)
    }

    fun fromPairing(data: PairingData, lastSeen: Long = 0L): Peer =
        Peer(
            id = data.desktopId,
            name = data.desktopName,
            secret = data.secret,
            host = data.host,
            port = data.port,
            lastSeen = lastSeen,
        )

    private fun saveAll(peers: List<Peer>) {
        val arr = JSONArray()
        for (p in peers) arr.put(peerToJson(p))
        prefs.edit().putString("peers", arr.toString()).apply()
    }

    private fun peerToJson(p: Peer): JSONObject =
        JSONObject()
            .put("id", p.id)
            .put("name", p.name)
            .put("secret", Base64.getEncoder().encodeToString(p.secret))
            .put("host", p.host)
            .put("port", p.port)
            .put("lastSeen", p.lastSeen)

    private fun peerFromJson(o: JSONObject): Peer {
        val secretB64 = o.getString("secret")
        val secret = Base64.getDecoder().decode(secretB64)
        return Peer(
            id = o.getString("id"),
            name = o.getString("name"),
            secret = secret,
            host = o.getString("host"),
            port = o.getInt("port"),
            lastSeen = o.getLong("lastSeen"),
        )
    }
}