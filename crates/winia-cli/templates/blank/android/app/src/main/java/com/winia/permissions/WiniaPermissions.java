package com.winia.permissions;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.provider.Settings;

import com.hjq.permissions.permission.PermissionLists;
import com.hjq.permissions.XXPermissions;
import com.hjq.permissions.permission.base.IPermission;

import java.util.concurrent.ConcurrentHashMap;

/**
 * Thin bridge between Winia's Rust permission API and XXPermissions.
 *
 * XXPermissions receives the result through its headless Fragment, so MainActivity does not
 * override onRequestPermissionsResult. Results are stored by request id and polled by Rust.
 */
public final class WiniaPermissions {
    private static final ConcurrentHashMap<Long, Integer> RESULTS = new ConcurrentHashMap<>();

    private static native void nativeOnResult(long requestId, boolean granted, boolean doNotAskAgain);

    private WiniaPermissions() {}

    public static void requestCamera(Activity activity, long requestId) {
        request(activity, requestId, PermissionLists.getCameraPermission());
    }

    public static void requestMicrophone(Activity activity, long requestId) {
        request(activity, requestId, PermissionLists.getRecordAudioPermission());
    }

    public static void requestLocation(Activity activity, long requestId) {
        request(activity, requestId,
                PermissionLists.getAccessFineLocationPermission(),
                PermissionLists.getAccessCoarseLocationPermission());
    }

    public static void requestNotifications(Activity activity, long requestId) {
        request(activity, requestId, PermissionLists.getPostNotificationsPermission());
    }

    public static void openSettings(Activity activity) {
        Intent intent = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS);
        intent.setData(Uri.parse("package:" + activity.getPackageName()));
        activity.startActivity(intent);
    }

    public static int takeResult(long requestId) {
        Integer value = RESULTS.remove(requestId);
        return value == null ? -1 : value;
    }

    private static void request(Activity activity, long requestId, IPermission... permissions) {
        XXPermissions.with(activity)
                .permissions(permissions)
                .request((granted, denied) -> {
                    boolean allGranted = denied.isEmpty();
                    int value;
                    if (allGranted) {
                        value = 1;
                    } else if (XXPermissions.isDoNotAskAgainPermissions(activity, denied)) {
                        value = 2;
                    } else {
                        value = 0;
                    }
                    RESULTS.put(requestId, value);
                    nativeOnResult(requestId, allGranted, value == 2);
                });
    }
}
