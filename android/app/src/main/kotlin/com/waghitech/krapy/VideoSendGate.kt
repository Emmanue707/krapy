package com.waghitech.krapy

object VideoSendGate {
    const val KEYFRAME: Int = Wire.VIDEO_FLAG_KEYFRAME
    const val MAX_AGE_US: Long = 60_000

    fun tooOld(flags: Int, ageUs: Long): Boolean = flags and KEYFRAME == 0 && ageUs > MAX_AGE_US

    fun shouldSend(flags: Int, dropUntilKeyframe: Boolean): Boolean {
        if (!dropUntilKeyframe) return true
        return flags and KEYFRAME != 0
    }

    fun onSent(flags: Int, dropUntilKeyframe: Boolean): Boolean {
        if (flags and KEYFRAME != 0) return false
        return dropUntilKeyframe
    }
}
