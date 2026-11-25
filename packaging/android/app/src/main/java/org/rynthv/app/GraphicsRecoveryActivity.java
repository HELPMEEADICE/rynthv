package org.rynthv.app;

import android.app.Activity;
import android.content.Intent;
import android.os.Binder;
import android.os.Bundle;
import android.os.IBinder;
import android.os.Process;
import android.os.RemoteException;
import android.view.Gravity;
import android.widget.TextView;

/** Starts a fresh native renderer after the previous process has disconnected. */
public final class GraphicsRecoveryActivity extends Activity {
    private static final String PROCESS = "renderer-process";
    private static final String LIFETIME = "renderer-lifetime";

    static void recover(RynthVActivity source) {
        Bundle extras = new Bundle();
        extras.putInt(PROCESS, Process.myPid());
        extras.putBinder(LIFETIME, new Binder());
        source.startActivity(new Intent(source, GraphicsRecoveryActivity.class)
            .putExtras(extras)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        TextView progress = new TextView(this);
        progress.setText("正在恢复图形设备和工作区…");
        progress.setGravity(Gravity.CENTER);
        setContentView(progress);
        Bundle extras = getIntent().getExtras();
        if (extras == null) {
            finish();
            return;
        }
        int pid = extras.getInt(PROCESS);
        IBinder lifetime = extras.getBinder(LIFETIME);
        if (pid <= 0 || pid == Process.myPid() || lifetime == null) {
            finish();
            return;
        }
        try {
            lifetime.linkToDeath(() -> runOnUiThread(this::launchEditor), 0);
            Process.killProcess(pid);
        } catch (RemoteException disconnected) {
            launchEditor();
        }
    }

    private void launchEditor() {
        startActivity(new Intent(this, RynthVActivity.class)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK));
        finishAndRemoveTask();
    }
}
