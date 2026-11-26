package org.rynthv.app

import android.app.ActivityManager
import android.app.AlertDialog
import android.app.NativeActivity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ComponentCallbacks2
import android.content.Intent
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.WindowManager
import android.widget.Toast
import android.window.OnBackInvokedDispatcher
import androidx.core.content.FileProvider
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray
import org.json.JSONObject

class RynthVActivity : NativeActivity() {
    companion object {
        init { System.loadLibrary("rynthv_android") }
        private const val PICK_DOCUMENT = 100
        private const val MULTIPLE = 1
        private const val TREE = 2
        private const val PUBLISH = 3
    }

    private val files = Executors.newSingleThreadExecutor()
    private val pickerResult = AtomicReference<String>()
    private lateinit var documents: DocumentStore
    private lateinit var playbackFocus: PlaybackFocus
    private lateinit var audioManager: AudioManager
    private var editorInput: EditorInput? = null
    private val routes = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(devices: Array<out AudioDeviceInfo>) = changed(devices)
        override fun onAudioDevicesRemoved(devices: Array<out AudioDeviceInfo>) = changed(devices)
        private fun changed(devices: Array<out AudioDeviceInfo>) {
            if (devices.any { it.isSink }) nativeEvent(5)
        }
    }
    private var pickerKind = 0
    private var pickerExtensions = ""
    private var publishSource: File? = null
    private var graphicsFailure: String? = null
    private var graphicsDialog: AlertDialog? = null

    private external fun nativeWake()
    private external fun nativeEvent(event: Int)

    override fun onCreate(state: Bundle?) {
        documents = DocumentStore(this)
        audioManager = getSystemService(AudioManager::class.java)
        playbackFocus = PlaybackFocus(audioManager, Runnable { nativeEvent(2) })
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
        super.onCreate(state)
        val input = EditorInput(this, Runnable { nativeEvent(6) })
        editorInput = input
        EditorWindow.install(this, input)
        files.execute { documents.pruneTransientFiles() }
        audioManager.registerAudioDeviceCallback(routes, Handler(Looper.getMainLooper()))
        volumeControlStream = AudioManager.STREAM_MUSIC
        if (Build.VERSION.SDK_INT >= 33) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT) { nativeEvent(3) }
        }
    }

    override fun onResume() {
        super.onResume()
        playbackFocus.setActive(true)
        nativeEvent(1)
        showGraphicsRecovery()
    }
    override fun onPause() {
        nativeEvent(0)
        playbackFocus.setActive(false)
        super.onPause()
    }
    override fun onBackPressed() { nativeEvent(3) }
    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (level == ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW ||
            level == ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL ||
            level >= ComponentCallbacks2.TRIM_MEMORY_MODERATE) nativeEvent(4)
    }
    override fun onLowMemory() { super.onLowMemory(); nativeEvent(4) }

    fun rynthvSetPlaybackWanted(wanted: Boolean): Boolean = playbackFocus.setWanted(wanted)
    fun rynthvUpdateKeyboard(purpose: Int, reset: Boolean) {
        runOnUiThread { editorInput?.showKeyboard(purpose, reset) }
    }
    fun rynthvTakeInput(): String? = editorInput?.takeEvents()
    fun rynthvReadClipboard(): String? {
        val clip = getSystemService(ClipboardManager::class.java).primaryClip
        if (clip == null || clip.itemCount == 0) return null
        return clip.getItemAt(0).text?.toString()
    }
    fun rynthvCopyText(text: String) {
        getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("RynthV", text))
    }
    fun rynthvFilesDirectory(): String = filesDir.absolutePath
    fun rynthvCacheDirectory(): String = cacheDir.absolutePath
    fun rynthvTotalMemory(): Long {
        val info = ActivityManager.MemoryInfo()
        getSystemService(ActivityManager::class.java).getMemoryInfo(info)
        return info.totalMem
    }
    fun rynthvMoveToBackground() { runOnUiThread { moveTaskToBack(true) } }
    fun rynthvShowFatalError(message: String) {
        Log.e("RynthV", message)
        runOnUiThread {
            AlertDialog.Builder(this).setTitle("RynthV").setMessage(message)
                .setPositiveButton(android.R.string.ok) { _, _ -> finish() }
                .setOnCancelListener { finish() }.show()
        }
    }
    fun rynthvGraphicsLost(message: String) {
        Log.e("RynthV", message)
        runOnUiThread { graphicsFailure = message; showGraphicsRecovery() }
    }
    private fun showGraphicsRecovery() {
        val failure = graphicsFailure ?: return
        if (graphicsDialog != null || isFinishing || isDestroyed || !hasWindowFocus()) return
        graphicsDialog = AlertDialog.Builder(this).setTitle(R.string.graphics_device_lost_title)
            .setMessage(getString(R.string.graphics_device_lost_message, failure))
            .setPositiveButton(R.string.graphics_recovery_action) { _, _ -> GraphicsRecoveryActivity.recover(this) }
            .setCancelable(false).show()
    }
    override fun onWindowFocusChanged(focused: Boolean) {
        super.onWindowFocusChanged(focused)
        if (focused) showGraphicsRecovery()
    }

    fun rynthvPick(kind: Int, title: String, extensions: String, source: String, mime: String) {
        runOnUiThread {
            pickerKind = kind
            pickerExtensions = extensions
            publishSource = if (kind == PUBLISH) File(source) else null
            try {
                val request = when (kind) {
                    TREE -> Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
                    PUBLISH -> Intent(Intent.ACTION_CREATE_DOCUMENT).setType(mime)
                        .addCategory(Intent.CATEGORY_OPENABLE)
                        .putExtra(Intent.EXTRA_TITLE, publishSource!!.name)
                    else -> Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*")
                        .addCategory(Intent.CATEGORY_OPENABLE)
                        .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, kind == MULTIPLE)
                }
                request.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
                if (kind == PUBLISH) request.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                startActivityForResult(request, PICK_DOCUMENT)
            } catch (error: RuntimeException) {
                complete(null, IllegalStateException(
                    getString(R.string.picker_error_message, title, error.message ?: error.javaClass.simpleName), error))
            }
        }
    }
    fun rynthvTakePickerResult(): String? = pickerResult.getAndSet(null)
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != PICK_DOCUMENT) return
        if (resultCode != RESULT_OK || data == null) { complete(arrayListOf(), null); return }
        val uris = LinkedHashSet<Uri>()
        data.data?.let { uris.add(it) }
        data.clipData?.let { clips ->
            for (index in 0 until clips.itemCount) clips.getItemAt(index).uri?.let { uris.add(it) }
        }
        val kind = pickerKind
        val extensions = pickerExtensions
        val source = publishSource
        publishSource = null
        if (uris.isEmpty()) {
            complete(null, IllegalArgumentException(getString(R.string.picker_no_document_returned)))
            return
        }
        files.execute {
            try {
                val paths = ArrayList<String>()
                for (uri in uris) {
                    when (kind) {
                        PUBLISH -> {
                            val file = requireNotNull(source)
                            documents.publish(file, uri)
                            paths.add(file.absolutePath)
                        }
                        TREE -> {
                            val access = data.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION
                            contentResolver.takePersistableUriPermission(uri, access)
                            paths.add(documents.importDirectory(uri).absolutePath)
                        }
                        else -> paths.add(documents.importFile(uri, extensions).absolutePath)
                    }
                    if (kind != MULTIPLE) break
                }
                complete(paths, null)
            } catch (error: Exception) { complete(null, error) }
        }
    }
    private fun complete(paths: ArrayList<String>?, error: Exception?) {
        val result = JSONObject()
        if (error == null) result.put("Ok", JSONArray(paths))
        else {
            Log.w("RynthV", "Document operation failed", error)
            result.put("Err", getString(R.string.file_operation_failed, error.message ?: error.javaClass.simpleName))
        }
        pickerResult.set(result.toString())
        nativeWake()
    }
    fun rynthvShare(path: String, mime: String) {
        files.execute {
            try {
                val shared = documents.prepareShare(File(path))
                val uri = FileProvider.getUriForFile(this, "$packageName.files", shared)
                val send = Intent(Intent.ACTION_SEND).setType(mime)
                    .putExtra(Intent.EXTRA_STREAM, uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                send.clipData = ClipData.newRawUri(shared.name, uri)
                runOnUiThread { startActivity(Intent.createChooser(send, null)) }
            } catch (error: Exception) {
                Log.w("RynthV", "Sharing failed", error)
                val message = getString(R.string.share_failed, error.message ?: error.javaClass.simpleName)
                runOnUiThread { Toast.makeText(this, message, Toast.LENGTH_LONG).show() }
            }
        }
    }
    override fun onDestroy() {
        audioManager.unregisterAudioDeviceCallback(routes)
        files.shutdown()
        super.onDestroy()
    }
}
