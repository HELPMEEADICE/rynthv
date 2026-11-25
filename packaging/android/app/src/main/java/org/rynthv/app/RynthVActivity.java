package org.rynthv.app;

import android.app.AlertDialog;
import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ComponentCallbacks2;
import android.content.Intent;
import android.media.AudioManager;
import android.media.AudioDeviceCallback;
import android.media.AudioDeviceInfo;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.window.OnBackInvokedDispatcher;
import android.util.Log;
import android.view.WindowManager;
import android.widget.Toast;
import androidx.core.content.FileProvider;
import java.io.File;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONArray;
import org.json.JSONObject;

public final class RynthVActivity extends NativeActivity {
    private static final int PICK_DOCUMENT = 100;
    private static final int MULTIPLE = 1, TREE = 2, PUBLISH = 3;
    private final ExecutorService files = Executors.newSingleThreadExecutor();
    private final AtomicReference<String> pickerResult = new AtomicReference<>();
    private DocumentStore documents;
    private PlaybackFocus playbackFocus;
    private AudioManager audioManager;
    private EditorInput editorInput;
    private final AudioDeviceCallback routes = new AudioDeviceCallback() {
        @Override public void onAudioDevicesAdded(AudioDeviceInfo[] devices) { changed(devices); }
        @Override public void onAudioDevicesRemoved(AudioDeviceInfo[] devices) { changed(devices); }

        private void changed(AudioDeviceInfo[] devices) {
            for (AudioDeviceInfo device : devices) {
                if (device.isSink()) {
                    nativeEvent(5);
                    return;
                }
            }
        }
    };
    private int pickerKind;
    private String pickerExtensions;
    private File publishSource;

    private static native void nativeWake();
    private static native void nativeEvent(int event);

    @Override
    protected void onCreate(Bundle state) {
        documents = new DocumentStore(this);
        audioManager = getSystemService(AudioManager.class);
        playbackFocus = new PlaybackFocus(audioManager, () -> nativeEvent(2));
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        super.onCreate(state);
        editorInput = new EditorInput(this, () -> nativeEvent(6));
        audioManager.registerAudioDeviceCallback(routes, new Handler(Looper.getMainLooper()));
        setVolumeControlStream(AudioManager.STREAM_MUSIC);
        if (Build.VERSION.SDK_INT >= 33) {
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT, () -> nativeEvent(3));
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        playbackFocus.setActive(true);
        nativeEvent(1);
    }

    @Override
    protected void onPause() {
        nativeEvent(0);
        playbackFocus.setActive(false);
        super.onPause();
    }

    @Override
    public void onBackPressed() {
        nativeEvent(3);
    }

    @Override
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        if (level == ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW
                || level == ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL
                || level >= ComponentCallbacks2.TRIM_MEMORY_MODERATE) nativeEvent(4);
    }

    @Override
    public void onLowMemory() {
        super.onLowMemory();
        nativeEvent(4);
    }

    boolean rynthvSetPlaybackWanted(boolean wanted) {
        return playbackFocus.setWanted(wanted);
    }

    void rynthvUpdateKeyboard(int purpose, boolean reset) {
        runOnUiThread(() -> editorInput.showKeyboard(purpose, reset));
    }

    String rynthvTakeInput() {
        return editorInput == null ? null : editorInput.takeEvents();
    }

    String rynthvReadClipboard() {
        ClipData clip = getSystemService(ClipboardManager.class).getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) return null;
        CharSequence text = clip.getItemAt(0).getText();
        return text == null ? null : text.toString();
    }

    void rynthvCopyText(String text) {
        getSystemService(ClipboardManager.class).setPrimaryClip(ClipData.newPlainText("RynthV", text));
    }

    String rynthvFilesDirectory() {
        return getFilesDir().getAbsolutePath();
    }

    void rynthvFinish() {
        runOnUiThread(this::finish);
    }

    void rynthvShowFatalError(String message) {
        Log.e("RynthV", message);
        runOnUiThread(() -> new AlertDialog.Builder(this)
            .setTitle("RynthV").setMessage(message)
            .setPositiveButton(android.R.string.ok, (dialog, which) -> finish())
            .setOnCancelListener(dialog -> finish()).show());
    }

    void rynthvPick(int kind, String title, String extensions, String source, String mime) {
        runOnUiThread(() -> {
            pickerKind = kind;
            pickerExtensions = extensions;
            publishSource = kind == PUBLISH ? new File(source) : null;
            try {
                Intent request;
                if (kind == TREE) {
                    request = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
                } else if (kind == PUBLISH) {
                    request = new Intent(Intent.ACTION_CREATE_DOCUMENT).setType(mime)
                        .addCategory(Intent.CATEGORY_OPENABLE)
                        .putExtra(Intent.EXTRA_TITLE, publishSource.getName());
                } else {
                    request = new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*")
                        .addCategory(Intent.CATEGORY_OPENABLE)
                        .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, kind == MULTIPLE);
                }
                request.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                if (kind == PUBLISH) request.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
                startActivityForResult(request, PICK_DOCUMENT);
            } catch (RuntimeException error) {
                complete(null, new IllegalStateException(title + "：" + error.getMessage(), error));
            }
        });
    }

    String rynthvTakePickerResult() {
        return pickerResult.getAndSet(null);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != PICK_DOCUMENT) return;
        if (resultCode != RESULT_OK || data == null) {
            complete(new ArrayList<>(), null);
            return;
        }
        Set<Uri> uris = new LinkedHashSet<>();
        if (data.getData() != null) uris.add(data.getData());
        ClipData clips = data.getClipData();
        if (clips != null) {
            for (int index = 0; index < clips.getItemCount(); index++) {
                Uri uri = clips.getItemAt(index).getUri();
                if (uri != null) uris.add(uri);
            }
        }
        int kind = pickerKind;
        String extensions = pickerExtensions;
        File source = publishSource;
        publishSource = null;
        if (uris.isEmpty()) {
            complete(null, new IllegalArgumentException("文件提供方没有返回文档。"));
            return;
        }
        files.execute(() -> {
            try {
                ArrayList<String> paths = new ArrayList<>();
                for (Uri uri : uris) {
                    if (kind == PUBLISH) {
                        documents.publish(source, uri);
                        paths.add(source.getAbsolutePath());
                    } else if (kind == TREE) {
                        int access = data.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION;
                        getContentResolver().takePersistableUriPermission(uri, access);
                        paths.add(documents.importDirectory(uri).getAbsolutePath());
                    } else {
                        paths.add(documents.importFile(uri, extensions).getAbsolutePath());
                    }
                    if (kind != MULTIPLE) break;
                }
                complete(paths, null);
            } catch (Exception error) {
                complete(null, error);
            }
        });
    }

    private void complete(ArrayList<String> paths, Exception error) {
        try {
            JSONObject result = new JSONObject();
            if (error == null) {
                result.put("Ok", new JSONArray(paths));
            } else {
                Log.w("RynthV", "Document operation failed", error);
                result.put("Err", "文件操作失败：" + error.getMessage());
            }
            pickerResult.set(result.toString());
        } catch (org.json.JSONException impossible) {
            throw new AssertionError(impossible);
        }
        nativeWake();
    }

    void rynthvShare(String path, String mime) {
        files.execute(() -> {
            try {
                File shared = documents.prepareShare(new File(path));
                Uri uri = FileProvider.getUriForFile(this, getPackageName() + ".files", shared);
                Intent send = new Intent(Intent.ACTION_SEND).setType(mime)
                    .putExtra(Intent.EXTRA_STREAM, uri)
                    .setClipData(ClipData.newRawUri(shared.getName(), uri))
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                runOnUiThread(() -> startActivity(Intent.createChooser(send, null)));
            } catch (Exception error) {
                Log.w("RynthV", "Sharing failed", error);
                runOnUiThread(() -> Toast.makeText(this, "分享失败：" + error.getMessage(),
                    Toast.LENGTH_LONG).show());
            }
        });
    }

    @Override
    protected void onDestroy() {
        audioManager.unregisterAudioDeviceCallback(routes);
        files.shutdown();
        super.onDestroy();
    }
}
