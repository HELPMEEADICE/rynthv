package org.rynthv.app

import android.app.Activity
import android.text.InputType
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import android.widget.FrameLayout
import androidx.core.graphics.Insets
import org.json.JSONArray
import org.json.JSONObject

class EditorInput(activity: Activity, private val notify: Runnable) : EditText(activity) {
    private val keyboard = activity.getSystemService(InputMethodManager::class.java)
    private var events = JSONArray()
    private var editing = false
    private var currentInputType = InputType.TYPE_CLASS_TEXT
    private var composition = ""

    init {
        setSingleLine(true)
        alpha = 0.01f
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        imeOptions = EditorInfo.IME_ACTION_DONE or EditorInfo.IME_FLAG_NO_EXTRACT_UI
        visibility = View.INVISIBLE
        activity.addContentView(this, FrameLayout.LayoutParams(1, 1, Gravity.TOP or Gravity.LEFT))
    }

    fun updateInsets(insets: Insets, windowHeight: Int) {
        val event = event("insets")
        event.put("left", insets.left).put("top", insets.top).put("right", insets.right)
            .put("bottom", insets.bottom).put("window_height", windowHeight)
        emit(event)
    }

    fun showKeyboard(purpose: Int, reset: Boolean) {
        if (purpose == 0) {
            if (editing) {
                editing = false
                composition = ""
                keyboard.hideSoftInputFromWindow(windowToken, 0)
                clearFocus()
                visibility = View.INVISIBLE
            }
            return
        }
        var type = InputType.TYPE_CLASS_TEXT
        if (purpose == 2) type = type or InputType.TYPE_TEXT_VARIATION_PASSWORD
        if (purpose == 3) type = type or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        var shouldReset = reset
        if (type != currentInputType) {
            currentInputType = type
            inputType = type
            shouldReset = true
        }
        if (shouldReset) {
            composition = ""
            keyboard.restartInput(this)
        }
        if (!editing) {
            editing = true
            visibility = View.VISIBLE
            requestFocus()
            post { if (editing) keyboard.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT) }
        }
    }

    @Synchronized fun takeEvents(): String? {
        if (events.length() == 0) return null
        val result = events.toString()
        events = JSONArray()
        return result
    }

    override fun onCreateInputConnection(info: EditorInfo): InputConnection? {
        val base = super.onCreateInputConnection(info) ?: return null
        return object : InputConnectionWrapper(base, false) {
            override fun setComposingText(text: CharSequence, cursor: Int): Boolean {
                composition = text.toString()
                emitText("preedit", composition)
                return true
            }
            override fun commitText(text: CharSequence, cursor: Int): Boolean {
                composition = ""
                emitText("commit", text.toString())
                return true
            }
            override fun finishComposingText(): Boolean {
                if (composition.isNotEmpty()) {
                    emitText("commit", composition)
                    composition = ""
                }
                return true
            }
            override fun deleteSurroundingText(before: Int, after: Int): Boolean {
                emit(event("delete").put("before", before).put("after", after))
                return true
            }
            override fun deleteSurroundingTextInCodePoints(before: Int, after: Int): Boolean =
                deleteSurroundingText(before, after)
            override fun performEditorAction(action: Int): Boolean {
                emitText("key", "Enter")
                return true
            }
        }
    }

    override fun onKeyDown(code: Int, key: KeyEvent): Boolean {
        val mapped = when (code) {
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> "Enter"
            KeyEvent.KEYCODE_DEL -> "Backspace"
            KeyEvent.KEYCODE_FORWARD_DEL -> "Delete"
            KeyEvent.KEYCODE_DPAD_LEFT -> "ArrowLeft"
            KeyEvent.KEYCODE_DPAD_RIGHT -> "ArrowRight"
            KeyEvent.KEYCODE_DPAD_UP -> "ArrowUp"
            KeyEvent.KEYCODE_DPAD_DOWN -> "ArrowDown"
            else -> null
        }
        if (mapped != null) { emitText("key", mapped); return true }
        val character = key.unicodeChar
        if (Character.isValidCodePoint(character) && character >= 32 && !key.isCtrlPressed) {
            emitText("text", String(Character.toChars(character)))
            return true
        }
        return super.onKeyDown(code, key)
    }

    @Synchronized private fun emit(event: JSONObject) {
        if (editing || event.optString("kind") == "insets") {
            events.put(event)
            notify.run()
        }
    }
    private fun emitText(kind: String, text: String) { emit(event(kind).put("text", text)) }
    private fun event(kind: String) = JSONObject().put("kind", kind)
}
