package org.rynthv.app

import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Handler
import android.os.Looper

class PlaybackFocus(private val manager: AudioManager, interrupt: Runnable) {
    private val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
        .setAudioAttributes(AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
        .setWillPauseWhenDucked(true)
        .setOnAudioFocusChangeListener({ change ->
            if (change < 0) {
                synchronized(this) { release() }
                interrupt.run()
            }
        }, Handler(Looper.getMainLooper())).build()
    private var active = false
    private var granted = false

    @Synchronized fun setActive(value: Boolean) {
        active = value
        if (!active) release()
    }

    @Synchronized fun setWanted(wanted: Boolean): Boolean {
        if (!wanted || !active) {
            release()
            return false
        }
        if (!granted) granted = manager.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        return granted
    }

    private fun release() {
        if (granted) {
            granted = false
            manager.abandonAudioFocusRequest(request)
        }
    }
}
