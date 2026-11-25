package org.rynthv.app

import android.app.Activity
import android.content.Intent
import android.os.Binder
import android.os.Bundle
import android.os.Process
import android.os.RemoteException
import android.view.Gravity
import android.widget.TextView

/** Starts a fresh native renderer after the previous process has disconnected. */
class GraphicsRecoveryActivity : Activity() {
    companion object {
        private const val PROCESS = "renderer-process"
        private const val LIFETIME = "renderer-lifetime"

        fun recover(source: RynthVActivity) {
            val extras = Bundle().apply {
                putInt(PROCESS, Process.myPid())
                putBinder(LIFETIME, Binder())
            }
            source.startActivity(Intent(source, GraphicsRecoveryActivity::class.java)
                .putExtras(extras).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        val progress = TextView(this).apply {
            text = "正在恢复图形设备和工作区…"
            gravity = Gravity.CENTER
        }
        setContentView(progress)
        val extras = intent.extras
        if (extras == null) { finish(); return }
        val pid = extras.getInt(PROCESS)
        val lifetime = extras.getBinder(LIFETIME)
        if (pid <= 0 || pid == Process.myPid() || lifetime == null) { finish(); return }
        try {
            lifetime.linkToDeath({ runOnUiThread { launchEditor() } }, 0)
            Process.killProcess(pid)
        } catch (disconnected: RemoteException) {
            launchEditor()
        }
    }

    private fun launchEditor() {
        startActivity(Intent(this, RynthVActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
        finishAndRemoveTask()
    }
}
