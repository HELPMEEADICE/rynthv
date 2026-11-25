package org.rynthv.app

import android.app.Activity
import android.os.Build
import android.view.WindowManager.LayoutParams
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat

object EditorWindow {
    fun install(activity: Activity, input: EditorInput) {
        val window = activity.window
        WindowCompat.setDecorFitsSystemWindows(window, false)
        if (Build.VERSION.SDK_INT >= 28) {
            val attrs = window.attributes
            attrs.layoutInDisplayCutoutMode = if (Build.VERSION.SDK_INT >= 30)
                LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            else LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            window.attributes = attrs
        }
        val decor = window.decorView
        ViewCompat.setOnApplyWindowInsetsListener(decor) { _, insets ->
            val occluded = insets.getInsets(WindowInsetsCompat.Type.systemBars() or
                WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime())
            val windowHeight = if (Build.VERSION.SDK_INT >= 30)
                activity.windowManager.currentWindowMetrics.bounds.height() else 0
            input.updateInsets(occluded, windowHeight)
            insets
        }
        ViewCompat.requestApplyInsets(decor)
    }
}
