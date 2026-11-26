package com.logosforgestudios.rustudio;

import android.app.NativeActivity;
import android.app.PendingIntent;
import android.content.ClipData;
import android.content.ComponentCallbacks2;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageInstaller;
import android.content.res.Configuration;
import android.net.Uri;
import android.provider.Settings;
import java.io.File;
import java.io.FileInputStream;
import java.io.OutputStream;
import android.graphics.drawable.ColorDrawable;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.widget.FrameLayout;

/**
 * Custom NativeActivity subclass that exposes a real Android InputConnection
 * directly to Rustudio's syntax-highlighted egui Script Editor (and other text
 * fields) with zero visible overlay views.
 *
 * This lets Gboard / Samsung Keyboard / SwiftKey deliver clipboard pastes,
 * swipe typing, smart backspace, and spacebar cursor sliding straight into
 * Rustudio's real editor buffer.
 */
public class RustudioActivity extends NativeActivity {
    private static final int REQ_UNKNOWN_SOURCES = 4107;
    private static final int REQ_SAF_CREATE_DOC = 0x5253;
    private static final int REQ_SAF_OPEN_DOC = 0x5254;
    private static final String ACTION_INSTALL_COMMIT = "com.logosforgestudios.rustudio.ACTION_INSTALL_COMMIT";
    private ImeProxyView imeView;
    private String pendingInstallApkPath = null;
    private BroadcastReceiver installReceiver = null;

    static {
        // Register librustudio_android.so with this ClassLoader so ART resolves
        // our nativeIme* JNI methods (NativeActivity's internal dlopen does not).
        System.loadLibrary("rustudio_android");
    }

    // Implemented in Rust (librustudio_android.so)
    static native void nativeImeCommitText(String text);
    static native void nativeImeDelete(int before, int after);
    static native void nativeImeKey(int keyCode, boolean down);
    static native void nativeImeAction(int action);
    static native void nativeOnLifecycle(int state);
    static native void nativeOnTrimMemory(int level);
    static native void nativeOnSafDocumentCreated(String uri);
    static native void nativeOnSafDocumentOpened(String uri);

    private void applyDarkFullscreenWindow() {
        Window w = getWindow();
        if (w == null) {
            return;
        }
        int darkBg = 0xFF1C1E22;
        w.clearFlags(WindowManager.LayoutParams.FLAG_TRANSLUCENT_STATUS
                | WindowManager.LayoutParams.FLAG_TRANSLUCENT_NAVIGATION);
        w.addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS
                | WindowManager.LayoutParams.FLAG_FULLSCREEN);
        // Prevent Android from panning the NativeActivity window upward off-screen
        // on API 30+ where WindowInsets.Type.ime reports keyboard height accurately.
        // On API 26-29, leave default resize behavior so system window insets report IME height.
        if (Build.VERSION.SDK_INT >= 30) {
            w.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING);
        }
        w.setBackgroundDrawable(new ColorDrawable(darkBg));
        w.setStatusBarColor(darkBg);
        w.setNavigationBarColor(darkBg);
        if (Build.VERSION.SDK_INT >= 29) {
            w.setStatusBarContrastEnforced(false);
            w.setNavigationBarContrastEnforced(false);
        }
        if (Build.VERSION.SDK_INT >= 30) {
            WindowManager.LayoutParams lp = w.getAttributes();
            lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
            w.setAttributes(lp);
            w.setDecorFitsSystemWindows(false);
        } else if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams lp = w.getAttributes();
            lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            w.setAttributes(lp);
        }
        View decor = w.getDecorView();
        if (decor != null) {
            decor.setBackgroundColor(darkBg);
            if (Build.VERSION.SDK_INT >= 30) {
                WindowInsetsController ctrl = decor.getWindowInsetsController();
                if (ctrl != null) {
                    ctrl.setSystemBarsAppearance(
                            0,
                            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                                    | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS
                    );
                    ctrl.setSystemBarsBehavior(
                            WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                    );
                    ctrl.hide(WindowInsets.Type.statusBars() | WindowInsets.Type.navigationBars());
                }
            } else {
                decor.setSystemUiVisibility(
                        View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                                | View.SYSTEM_UI_FLAG_FULLSCREEN
                                | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                );
            }
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        applyDarkFullscreenWindow();
        imeView = new ImeProxyView(this);
        imeView.setBackgroundColor(0);
        imeView.setFocusable(true);
        imeView.setFocusableInTouchMode(true);
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(1, 1);
        addContentView(imeView, lp);
    }

    @Override
    protected void onResume() {
        super.onResume();
        nativeOnLifecycle(3);
        applyDarkFullscreenWindow();
        maybeResumePendingInstall();
    }

    @Override
    protected void onPause() {
        nativeOnLifecycle(1);
        super.onPause();
    }

    @Override
    protected void onStop() {
        nativeOnLifecycle(2);
        super.onStop();
    }

    @Override
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        if (level >= ComponentCallbacks2.TRIM_MEMORY_UI_HIDDEN) {
            nativeOnTrimMemory(level);
        }
    }

    @Override
    public void onLowMemory() {
        super.onLowMemory();
        nativeOnTrimMemory(ComponentCallbacks2.TRIM_MEMORY_COMPLETE);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_UNKNOWN_SOURCES) {
            maybeResumePendingInstall();
        } else if (requestCode == REQ_SAF_CREATE_DOC) {
            String uri = null;
            if (resultCode == RESULT_OK && data != null && data.getData() != null) {
                uri = data.getData().toString();
            }
            nativeOnSafDocumentCreated(uri);
        } else if (requestCode == REQ_SAF_OPEN_DOC) {
            if (resultCode == RESULT_OK && data != null) {
                ClipData clip = data.getClipData();
                if (clip != null && clip.getItemCount() > 0) {
                    for (int i = 0; i < clip.getItemCount(); i++) {
                        ClipData.Item item = clip.getItemAt(i);
                        if (item != null && item.getUri() != null) {
                            nativeOnSafDocumentOpened(item.getUri().toString());
                        }
                    }
                } else if (data.getData() != null) {
                    nativeOnSafDocumentOpened(data.getData().toString());
                }
            }
        }
    }

    @Override
    protected void onDestroy() {
        if (installReceiver != null) {
            try {
                unregisterReceiver(installReceiver);
            } catch (Throwable ignored) {
            }
            installReceiver = null;
        }
        super.onDestroy();
    }

    private void ensureInstallReceiver() {
        if (installReceiver != null) {
            return;
        }
        installReceiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                if (intent == null) {
                    return;
                }
                int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                    if (confirm != null) {
                        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                        try {
                            startActivity(confirm);
                        } catch (Throwable ignored) {
                        }
                    }
                }
            }
        };
        IntentFilter filter = new IntentFilter(ACTION_INSTALL_COMMIT);
        if (Build.VERSION.SDK_INT >= 33) {
            registerReceiver(installReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(installReceiver, filter);
        }
    }

    private void maybeResumePendingInstall() {
        final String path = pendingInstallApkPath;
        if (path == null) {
            return;
        }
        if (Build.VERSION.SDK_INT < 26 || getPackageManager().canRequestPackageInstalls()) {
            pendingInstallApkPath = null;
            new Thread(new Runnable() {
                @Override
                public void run() {
                    installApk(path);
                }
            }, "rustudio-apk-install").start();
        }
    }

    /**
     * Installs an APK from app-private cache using Android's PackageInstaller session API.
     * Requests "Install unknown apps" permission first if not yet granted, and automatically
     * resumes installation when the user returns from Settings.
     */
    public String installApk(String apkPath) {
        if (apkPath == null || apkPath.isEmpty()) {
            return "error:Empty APK path";
        }
        File apk = new File(apkPath);
        if (!apk.exists() || !apk.isFile() || apk.length() <= 0) {
            return "error:APK file not found";
        }
        if (Build.VERSION.SDK_INT >= 26 && !getPackageManager().canRequestPackageInstalls()) {
            pendingInstallApkPath = apkPath;
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    try {
                        Intent intent = new Intent(
                                Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                                Uri.parse("package:" + getPackageName())
                        );
                        startActivityForResult(intent, REQ_UNKNOWN_SOURCES);
                    } catch (Throwable ignored) {
                    }
                }
            });
            return "permission_requested";
        }
        ensureInstallReceiver();
        PackageInstaller.Session session = null;
        try {
            PackageInstaller installer = getPackageManager().getPackageInstaller();
            PackageInstaller.SessionParams params =
                    new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
            params.setAppPackageName(getPackageName());
            params.setSize(apk.length());
            int sessionId = installer.createSession(params);
            session = installer.openSession(sessionId);
            try (FileInputStream in = new FileInputStream(apk);
                 OutputStream out = session.openWrite("rustudio_update.apk", 0, apk.length())) {
                byte[] buf = new byte[65536];
                int n;
                while ((n = in.read(buf)) > 0) {
                    out.write(buf, 0, n);
                }
                session.fsync(out);
            }
            Intent commitIntent = new Intent(ACTION_INSTALL_COMMIT);
            commitIntent.setPackage(getPackageName());
            int flags = PendingIntent.FLAG_UPDATE_CURRENT;
            if (Build.VERSION.SDK_INT >= 31) {
                flags |= PendingIntent.FLAG_MUTABLE;
            }
            PendingIntent pi = PendingIntent.getBroadcast(this, sessionId, commitIntent, flags);
            session.commit(pi.getIntentSender());
            session.close();
            session = null;
            return "installing";
        } catch (Throwable t) {
            if (session != null) {
                try {
                    session.abandon();
                } catch (Throwable ignored) {
                }
            }
            String msg = t.getMessage();
            return "error:" + t.getClass().getSimpleName() + (msg != null ? (": " + msg) : "");
        }
    }

    /**
     * Relaunches the app in a fresh task and terminates the current process.
     */
    public void restartApp() {
        Intent intent = getPackageManager().getLaunchIntentForPackage(getPackageName());
        if (intent != null) {
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
            startActivity(intent);
        }
        finishAffinity();
        Runtime.getRuntime().exit(0);
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            applyDarkFullscreenWindow();
        }
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        applyDarkFullscreenWindow();
    }

    @Override
    public void onBackPressed() {
        // Handled natively as Esc in Rust (src/android/lifecycle.rs); never finish() on Back.
    }

    /**
     * Called from Rust (src/android/keyboard.rs) on the Java main thread when
     * an egui TextEdit or Roblox TextBox gains or loses focus.
     */
    public void setNativeImeOpen(final boolean open) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (imeView == null) {
                    return;
                }
                InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm == null) {
                    return;
                }
                if (open) {
                    imeView.resetComposing();
                    imeView.requestFocus();
                    imm.restartInput(imeView);
                    imm.showSoftInput(imeView, InputMethodManager.SHOW_IMPLICIT);
                    applyDarkFullscreenWindow();
                } else {
                    imeView.resetComposing();
                    imeView.clearFocus();
                    imm.hideSoftInputFromWindow(imeView.getWindowToken(), 0);
                    applyDarkFullscreenWindow();
                }
            }
        });
    }

    /**
     * Called from Rust (src/android/keyboard.rs) when focus moves between
     * text fields while the soft keyboard remains open.
     */
    public void resetImeComposing() {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (imeView == null) {
                    return;
                }
                imeView.resetComposing();
                InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm != null) {
                    imm.restartInput(imeView);
                }
            }
        });
    }

    private static final class ImeProxyView extends View {
        private int composingLen = 0;

        ImeProxyView(Context ctx) {
            super(ctx);
        }

        void resetComposing() {
            composingLen = 0;
        }

        @Override
        public boolean onTouchEvent(MotionEvent event) {
            // Never intercept touches from NativeActivity's Vulkan/GLES surface.
            return false;
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
            outAttrs.inputType = InputType.TYPE_CLASS_TEXT
                    | InputType.TYPE_TEXT_FLAG_MULTI_LINE
                    | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
            outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI
                    | EditorInfo.IME_FLAG_NO_FULLSCREEN
                    | EditorInfo.IME_ACTION_NONE;
            outAttrs.initialSelStart = -1;
            outAttrs.initialSelEnd = -1;
            return new RustudioInputConnection(this);
        }

        private final class RustudioInputConnection extends BaseInputConnection {
            RustudioInputConnection(View targetView) {
                super(targetView, false);
            }

            @Override
            public boolean commitText(CharSequence text, int newCursorPosition) {
                if (composingLen > 0) {
                    nativeImeDelete(composingLen, 0);
                    composingLen = 0;
                }
                if (text != null && text.length() > 0) {
                    nativeImeCommitText(text.toString());
                }
                return true;
            }

            @Override
            public boolean setComposingText(CharSequence text, int newCursorPosition) {
                if (composingLen > 0) {
                    nativeImeDelete(composingLen, 0);
                    composingLen = 0;
                }
                if (text != null && text.length() > 0) {
                    String s = text.toString();
                    nativeImeCommitText(s);
                    composingLen = s.codePointCount(0, s.length());
                }
                return true;
            }

            @Override
            public boolean finishComposingText() {
                composingLen = 0;
                return true;
            }

            @Override
            public boolean deleteSurroundingText(int beforeLength, int afterLength) {
                if (composingLen > 0) {
                    composingLen = Math.max(0, composingLen - beforeLength);
                }
                if (beforeLength > 0 || afterLength > 0) {
                    nativeImeDelete(beforeLength, afterLength);
                }
                return true;
            }

            @Override
            public boolean performEditorAction(int editorAction) {
                composingLen = 0;
                nativeImeKey(KeyEvent.KEYCODE_ENTER, true);
                nativeImeKey(KeyEvent.KEYCODE_ENTER, false);
                return true;
            }

            @Override
            public boolean performContextMenuAction(int id) {
                if (id == android.R.id.selectAll) {
                    nativeImeAction(1);
                    return true;
                } else if (id == android.R.id.cut) {
                    nativeImeAction(2);
                    return true;
                } else if (id == android.R.id.copy) {
                    nativeImeAction(3);
                    return true;
                } else if (id == android.R.id.paste || id == android.R.id.pasteAsPlainText) {
                    nativeImeAction(4);
                    return true;
                }
                return super.performContextMenuAction(id);
            }

            @Override
            public boolean sendKeyEvent(KeyEvent event) {
                int code = event.getKeyCode();
                int action = event.getAction();
                boolean down = (action == KeyEvent.ACTION_DOWN);
                boolean up = (action == KeyEvent.ACTION_UP);

                // Let system navigation & volume keys pass through to Android / NativeActivity.
                if (code == KeyEvent.KEYCODE_BACK
                        || code == KeyEvent.KEYCODE_VOLUME_UP
                        || code == KeyEvent.KEYCODE_VOLUME_DOWN
                        || code == KeyEvent.KEYCODE_VOLUME_MUTE) {
                    return super.sendKeyEvent(event);
                }

                // Navigation / editing keys handled directly by Rust
                if (code == KeyEvent.KEYCODE_DEL
                        || code == KeyEvent.KEYCODE_FORWARD_DEL
                        || code == KeyEvent.KEYCODE_ENTER
                        || code == KeyEvent.KEYCODE_NUMPAD_ENTER
                        || code == KeyEvent.KEYCODE_TAB
                        || code == KeyEvent.KEYCODE_DPAD_LEFT
                        || code == KeyEvent.KEYCODE_DPAD_RIGHT
                        || code == KeyEvent.KEYCODE_DPAD_UP
                        || code == KeyEvent.KEYCODE_DPAD_DOWN
                        || code == KeyEvent.KEYCODE_MOVE_HOME
                        || code == KeyEvent.KEYCODE_MOVE_END) {
                    if (down && code == KeyEvent.KEYCODE_DEL && composingLen > 0) {
                        composingLen = Math.max(0, composingLen - 1);
                    }
                    if (down || up) {
                        nativeImeKey(code, down);
                    }
                    return true;
                }

                // Printable characters sent as KeyEvent (some OEM keyboards)
                if (down) {
                    int uni = event.getUnicodeChar();
                    if (uni != 0 && !Character.isISOControl(uni)) {
                        if (composingLen > 0) {
                            nativeImeDelete(composingLen, 0);
                            composingLen = 0;
                        }
                        nativeImeCommitText(new String(Character.toChars(uni)));
                        return true;
                    }
                } else if (up) {
                    int uni = event.getUnicodeChar();
                    if (uni != 0 && !Character.isISOControl(uni)) {
                        return true;
                    }
                }

                return super.sendKeyEvent(event);
            }
        }
    }
}
