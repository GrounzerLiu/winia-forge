package com.winia.back;

import android.app.Activity;
import android.os.Build;
import android.window.BackEvent;
import android.window.OnBackAnimationCallback;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;

/**
 * The Java half of the Android back bridge.
 *
 * Rust attaches this class to the GameActivity and calls {@link #setEnabled(boolean)} whenever the
 * number of enabled handlers crosses zero. Registration is therefore dynamic: while nothing can
 * consume back, no system callback exists and the platform keeps its own back animation.
 */
public final class WiniaBack {
    private static Activity activity;
    private static OnBackInvokedCallback callback;
    private static boolean enabled;

    private WiniaBack() {}

    public static void attach(Activity owner) {
        activity = owner;
        // A new Activity starts unregistered; Rust re-enables it when handlers exist.
        enabled = false;
    }

    public static void detach() {
        setEnabled(false);
        nativeBackDetached();
        activity = null;
    }

    public static void setEnabled(final boolean value) {
        if (Build.VERSION.SDK_INT < 33) {
            return;
        }
        final Activity owner = activity;
        if (owner == null || owner.isDestroyed()) {
            return;
        }
        owner.runOnUiThread(() -> applyEnabled(owner, value));
    }

    private static void applyEnabled(Activity owner, boolean value) {
        if (owner.isDestroyed()) {
            return;
        }
        if (value == enabled) {
            return;
        }
        enabled = value;
        if (value) {
            final float density = owner.getResources().getDisplayMetrics().density;
            if (Build.VERSION.SDK_INT >= 34) {
                callback = new OnBackAnimationCallback() {
                    @Override
                    public void onBackStarted(BackEvent event) {
                        nativeBackStarted(
                                event.getTouchX() / density,
                                event.getTouchY() / density,
                                event.getProgress(),
                                event.getSwipeEdge(),
                                frameTime(event));
                    }

                    @Override
                    public void onBackProgressed(BackEvent event) {
                        nativeBackProgressed(
                                event.getTouchX() / density,
                                event.getTouchY() / density,
                                event.getProgress(),
                                event.getSwipeEdge(),
                                frameTime(event));
                    }

                    @Override
                    public void onBackCancelled() {
                        nativeBackCancelled();
                    }

                    @Override
                    public void onBackInvoked() {
                        onInvoked();
                    }
                };
            } else {
                callback = new OnBackInvokedCallback() {
                    @Override
                    public void onBackInvoked() {
                        onInvoked();
                    }
                };
            }
            owner.getOnBackInvokedDispatcher()
                    .registerOnBackInvokedCallback(
                            OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback);
        } else if (callback != null) {
            owner.getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(callback);
            callback = null;
        }
    }

    private static long frameTime(BackEvent event) {
        return Build.VERSION.SDK_INT >= 36 ? event.getFrameTimeMillis() : 0L;
    }

    private static void onInvoked() {
        if (!nativeHasHandlers()) {
            finishActivity();
            return;
        }
        nativeBackCompleted();
    }

    /** Rust fallback when a completion raced the handler table: finish the activity. */
    public static void finishActivity() {
        Activity owner = activity;
        if (owner != null) {
            owner.runOnUiThread(owner::finish);
        }
    }

    /** Called by MainActivity.onBackPressed() on API < 33 (the system still uses that path). */
    public static boolean dispatchLegacyBack() {
        if (Build.VERSION.SDK_INT >= 33) {
            return false;
        }
        if (!nativeHasHandlers()) {
            return false;
        }
        nativeBackCompleted();
        return true;
    }

    private static native void nativeBackStarted(
            float touchX, float touchY, float progress, int edge, long frameMillis);

    private static native void nativeBackProgressed(
            float touchX, float touchY, float progress, int edge, long frameMillis);

    private static native void nativeBackCancelled();

    private static native void nativeBackCompleted();

    private static native boolean nativeHasHandlers();

    private static native void nativeBackDetached();
}
