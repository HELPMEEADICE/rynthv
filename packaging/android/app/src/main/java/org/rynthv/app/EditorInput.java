package org.rynthv.app;

import android.app.Activity;
import android.text.InputType;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputConnectionWrapper;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;
import androidx.core.graphics.Insets;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

final class EditorInput extends EditText {
    private final InputMethodManager keyboard;
    private final Runnable notify;
    private JSONArray events = new JSONArray();
    private boolean editing;
    private int inputType = InputType.TYPE_CLASS_TEXT;
    private String composition = "";

    EditorInput(Activity activity, Runnable notify) {
        super(activity);
        keyboard = activity.getSystemService(InputMethodManager.class);
        this.notify = notify;
        setSingleLine(true);
        setAlpha(0.01f);
        setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        setVisibility(View.INVISIBLE);
        activity.addContentView(this, new FrameLayout.LayoutParams(1, 1, Gravity.TOP | Gravity.LEFT));
    }

    void updateInsets(Insets insets, int windowHeight) {
        JSONObject event = event("insets");
        put(event, "left", insets.left);
        put(event, "top", insets.top);
        put(event, "right", insets.right);
        put(event, "bottom", insets.bottom);
        put(event, "window_height", windowHeight);
        emit(event);
    }

    void showKeyboard(int purpose, boolean reset) {
        if (purpose == 0) {
            if (editing) {
                editing = false;
                composition = "";
                keyboard.hideSoftInputFromWindow(getWindowToken(), 0);
                clearFocus();
                setVisibility(View.INVISIBLE);
            }
            return;
        }
        int type = InputType.TYPE_CLASS_TEXT;
        if (purpose == 2) type |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
        if (purpose == 3) type |= InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
        if (type != inputType) {
            inputType = type;
            setInputType(type);
            reset = true;
        }
        if (reset) {
            composition = "";
            keyboard.restartInput(this);
        }
        if (!editing) {
            editing = true;
            setVisibility(View.VISIBLE);
            requestFocus();
            post(() -> {
                if (editing) keyboard.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT);
            });
        }
    }

    synchronized String takeEvents() {
        if (events.length() == 0) return null;
        String result = events.toString();
        events = new JSONArray();
        return result;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo info) {
        InputConnection base = super.onCreateInputConnection(info);
        if (base == null) return null;
        return new InputConnectionWrapper(base, false) {
            @Override
            public boolean setComposingText(CharSequence text, int cursor) {
                composition = text.toString();
                emitText("preedit", composition);
                return true;
            }

            @Override
            public boolean commitText(CharSequence text, int cursor) {
                composition = "";
                emitText("commit", text.toString());
                return true;
            }

            @Override
            public boolean finishComposingText() {
                if (!composition.isEmpty()) {
                    emitText("commit", composition);
                    composition = "";
                }
                return true;
            }

            @Override
            public boolean deleteSurroundingText(int before, int after) {
                JSONObject event = event("delete");
                put(event, "before", before);
                put(event, "after", after);
                emit(event);
                return true;
            }

            @Override
            public boolean deleteSurroundingTextInCodePoints(int before, int after) {
                return deleteSurroundingText(before, after);
            }

            @Override
            public boolean performEditorAction(int action) {
                emitText("key", "Enter");
                return true;
            }
        };
    }

    @Override
    public boolean onKeyDown(int code, KeyEvent key) {
        String mapped = switch (code) {
            case KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> "Enter";
            case KeyEvent.KEYCODE_DEL -> "Backspace";
            case KeyEvent.KEYCODE_FORWARD_DEL -> "Delete";
            case KeyEvent.KEYCODE_DPAD_LEFT -> "ArrowLeft";
            case KeyEvent.KEYCODE_DPAD_RIGHT -> "ArrowRight";
            case KeyEvent.KEYCODE_DPAD_UP -> "ArrowUp";
            case KeyEvent.KEYCODE_DPAD_DOWN -> "ArrowDown";
            default -> null;
        };
        if (mapped != null) {
            emitText("key", mapped);
            return true;
        }
        int character = key.getUnicodeChar();
        if (Character.isValidCodePoint(character) && character >= 32 && !key.isCtrlPressed()) {
            emitText("text", new String(Character.toChars(character)));
            return true;
        }
        return super.onKeyDown(code, key);
    }

    private synchronized void emit(JSONObject event) {
        if (editing || "insets".equals(event.optString("kind"))) {
            events.put(event);
            notify.run();
        }
    }

    private void emitText(String kind, String text) {
        JSONObject event = event(kind);
        put(event, "text", text);
        emit(event);
    }

    private static JSONObject event(String kind) {
        JSONObject event = new JSONObject();
        put(event, "kind", kind);
        return event;
    }

    private static void put(JSONObject object, String key, Object value) {
        try {
            object.put(key, value);
        } catch (JSONException impossible) {
            throw new AssertionError(impossible);
        }
    }
}
