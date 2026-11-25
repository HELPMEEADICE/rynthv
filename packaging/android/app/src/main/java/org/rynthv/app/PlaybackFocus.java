package org.rynthv.app;

import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.os.Handler;
import android.os.Looper;

final class PlaybackFocus {
    private final AudioManager manager;
    private final AudioFocusRequest request;
    private boolean active;
    private boolean granted;

    PlaybackFocus(AudioManager manager, Runnable interrupt) {
        this.manager = manager;
        request = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build())
            .setWillPauseWhenDucked(true)
            .setOnAudioFocusChangeListener(change -> {
                if (change < 0) {
                    synchronized (this) {
                        release();
                    }
                    interrupt.run();
                }
            }, new Handler(Looper.getMainLooper()))
            .build();
    }

    synchronized void setActive(boolean value) {
        active = value;
        if (!active) release();
    }

    synchronized boolean setWanted(boolean wanted) {
        if (!wanted || !active) {
            release();
            return false;
        }
        if (!granted) {
            granted = manager.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED;
        }
        return granted;
    }

    private void release() {
        if (granted) {
            granted = false;
            manager.abandonAudioFocusRequest(request);
        }
    }
}
