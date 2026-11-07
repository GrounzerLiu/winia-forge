package com.winia.permissions;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;

import com.hjq.permissions.XXPermissions;
import com.hjq.permissions.permission.PermissionLists;
import com.hjq.permissions.permission.base.IPermission;
import com.hjq.permissions.tools.PermissionSettingPage;

import java.util.List;
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

    public static void requestCapability(Activity activity, long requestId, String capability) {
        request(activity, requestId, permissionsFor(capability));
    }

    public static boolean isGranted(Activity activity, String capability) {
        if ("notifications".equals(capability) && Build.VERSION.SDK_INT < 33) {
            return true;
        }
        if ("location".equals(capability)) {
            return XXPermissions.isGrantedPermission(activity, PermissionLists.getAccessFineLocationPermission())
                    || XXPermissions.isGrantedPermission(activity, PermissionLists.getAccessCoarseLocationPermission());
        }
        return XXPermissions.isGrantedPermissions(activity, permissionsFor(capability));
    }

    public static void openSettingsFor(Activity activity, String capability) {
        IPermission[] permissions = permissionsFor(capability);
        List<Intent> intents = PermissionSettingPage.getCommonPermissionSettingIntent(activity, permissions);
        Intent intent = intents.isEmpty()
                ? PermissionSettingPage.getApplicationDetailsSettingsIntent(activity, permissions)
                : intents.get(0);
        activity.startActivity(intent);
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

    private static IPermission[] permissionsFor(String capability) {
        switch (capability) {
            case "camera":
                return new IPermission[]{PermissionLists.getCameraPermission()};
            case "microphone":
                return new IPermission[]{PermissionLists.getRecordAudioPermission()};
            case "location":
                return new IPermission[]{
                        PermissionLists.getAccessFineLocationPermission(),
                        PermissionLists.getAccessCoarseLocationPermission()};
            case "notifications":
                return new IPermission[]{PermissionLists.getPostNotificationsPermission()};
            case "systemAlertWindow":
                return new IPermission[]{PermissionLists.getSystemAlertWindowPermission()};
            case "writeSettings":
                return new IPermission[]{PermissionLists.getWriteSettingsPermission()};
            case "manageExternalStorage":
                return new IPermission[]{PermissionLists.getManageExternalStoragePermission()};
            case "requestInstallPackages":
                return new IPermission[]{PermissionLists.getRequestInstallPackagesPermission()};
            case "scheduleExactAlarm":
                return new IPermission[]{PermissionLists.getScheduleExactAlarmPermission()};
            case "usageStats":
                return new IPermission[]{PermissionLists.getPackageUsageStatsPermission()};
            case "ignoreBatteryOptimizations":
                return new IPermission[]{PermissionLists.getRequestIgnoreBatteryOptimizationsPermission()};
            default:
                throw new IllegalArgumentException("unknown permission capability: " + capability);
        }
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
