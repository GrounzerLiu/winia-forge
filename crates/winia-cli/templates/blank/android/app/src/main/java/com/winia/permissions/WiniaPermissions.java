package com.winia.permissions;

import android.accessibilityservice.AccessibilityService;
import android.app.Activity;
import android.app.admin.DeviceAdminReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.net.VpnService;
import android.os.Build;
import android.os.Looper;
import android.provider.Settings;
import android.service.notification.NotificationListenerService;

import com.hjq.permissions.XXPermissions;
import com.hjq.permissions.permission.PermissionLists;
import com.hjq.permissions.permission.base.IPermission;
import com.hjq.permissions.tools.PermissionSettingPage;

import java.util.ArrayDeque;
import java.util.List;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Thin bridge between Winia's Rust permission API and XXPermissions.
 *
 * XXPermissions receives the result through its headless Fragment, so MainActivity does not
 * override onRequestPermissionsResult. Results are stored by request id and polled by Rust.
 */
public final class WiniaPermissions {
    private static final ConcurrentHashMap<Long, Integer> RESULTS = new ConcurrentHashMap<>();
    private static final Set<Long> CANCELLED = ConcurrentHashMap.newKeySet();
    private static final Set<Long> PENDING = ConcurrentHashMap.newKeySet();
    private static final ArrayDeque<RequestTask> QUEUE = new ArrayDeque<>();
    private static final Object QUEUE_LOCK = new Object();
    private static boolean ACTIVE;
    private static final String PREFS = "winia_permissions";

    private static final class RequestTask {
        final Activity activity;
        final Runnable runnable;

        RequestTask(Activity activity, Runnable runnable) {
            this.activity = activity;
            this.runnable = runnable;
        }
    }

    private static native void nativeOnResult(long requestId, boolean granted, boolean doNotAskAgain);

    private WiniaPermissions() {}

    public static void requestCapability(Activity activity, long requestId, String capability) {
        markRequested(activity, capability);
        IPermission[] permissions = permissionsFor(capability);
        enqueue(activity, () -> request(activity, requestId, capability, permissions));
    }

    public static void requestService(Activity activity, long requestId, String kind, String className) {
        IPermission permission = servicePermission(kind, className);
        enqueue(activity, () -> request(activity, requestId, "", permission));
    }

    public static boolean isServiceGranted(Activity activity, String kind, String className) {
        return XXPermissions.isGrantedPermission(activity, servicePermission(kind, className));
    }

    public static boolean isMainThread() {
        return Looper.myLooper() == Looper.getMainLooper();
    }

    public static boolean hasRequested(Activity activity, String capability) {
        return prefs(activity).getBoolean(capability, false);
    }

    public static void markRequested(Activity activity, String capability) {
        prefs(activity).edit().putBoolean(capability, true).apply();
    }

    private static SharedPreferences prefs(Activity activity) {
        return activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    public static boolean isPermanentlyDenied(Activity activity, String capability) {
        return XXPermissions.isDoNotAskAgainPermissions(activity, permissionsFor(capability));
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
        markRequested(activity, capability);
        IPermission[] permissions = permissionsFor(capability);
        List<Intent> intents = PermissionSettingPage.getCommonPermissionSettingIntent(activity, permissions);
        Intent intent = intents.isEmpty()
                ? PermissionSettingPage.getApplicationDetailsSettingsIntent(activity, permissions)
                : intents.get(0);
        activity.startActivity(intent);
    }

    public static void openSettingsForService(Activity activity, String kind, String className) {
        IPermission permission = servicePermission(kind, className);
        List<Intent> intents = PermissionSettingPage.getCommonPermissionSettingIntent(activity, permission);
        Intent intent = intents.isEmpty()
                ? PermissionSettingPage.getApplicationDetailsSettingsIntent(activity, permission)
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

    public static void clearResult(long requestId) {
        if (PENDING.remove(requestId)) {
            CANCELLED.add(requestId);
        } else {
            RESULTS.remove(requestId);
        }
    }

    private static IPermission servicePermission(String kind, String className) {
        switch (kind) {
            case "accessibility":
                return PermissionLists.getBindAccessibilityServicePermission(
                        serviceClass(className, AccessibilityService.class));
            case "notificationListener":
                return PermissionLists.getBindNotificationListenerServicePermission(
                        serviceClass(className, NotificationListenerService.class));
            case "vpn":
                return PermissionLists.getBindVpnServicePermission();
            case "deviceAdmin":
                return PermissionLists.getBindDeviceAdminPermission(
                        serviceClass(className, DeviceAdminReceiver.class));
            default:
                throw new IllegalArgumentException("unknown service permission kind: " + kind);
        }
    }

    private static <T> Class<? extends T> serviceClass(String name, Class<T> base) {
        try {
            return Class.forName(name).asSubclass(base);
        } catch (ClassNotFoundException error) {
            throw new IllegalArgumentException("service class not found: " + name, error);
        }
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
            case "backgroundLocation":
                return new IPermission[]{PermissionLists.getAccessBackgroundLocationPermission()};
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
            case "pictureInPicture":
                return new IPermission[]{PermissionLists.getPictureInPicturePermission()};
            case "useFullScreenIntent":
                return new IPermission[]{PermissionLists.getUseFullScreenIntentPermission()};
            case "accessNotificationPolicy":
                return new IPermission[]{PermissionLists.getAccessNotificationPolicyPermission()};
            case "manageMedia":
                return new IPermission[]{PermissionLists.getManageMediaPermission()};
            case "getInstalledApps":
                return new IPermission[]{PermissionLists.getGetInstalledAppsPermission()};
            default:
                throw new IllegalArgumentException("unknown permission capability: " + capability);
        }
    }

    private static void enqueue(Activity activity, Runnable task) {
        synchronized (QUEUE_LOCK) {
            if (ACTIVE) {
                QUEUE.add(new RequestTask(activity, task));
                return;
            }
            ACTIVE = true;
        }
        activity.runOnUiThread(task);
    }

    private static void finish(Activity activity) {
        RequestTask next;
        synchronized (QUEUE_LOCK) {
            next = QUEUE.poll();
            if (next == null) {
                ACTIVE = false;
                return;
            }
        }
        next.activity.runOnUiThread(next.runnable);
    }

    private static void request(Activity activity, long requestId, String capability, IPermission... permissions) {
        PENDING.add(requestId);
        XXPermissions.with(activity)
                .permissions(permissions)
                .request((granted, denied) -> {
                    PENDING.remove(requestId);
                    if (CANCELLED.remove(requestId)) {
                        finish(activity);
                        return;
                    }
                    boolean allGranted = denied.isEmpty();
                    if ("location".equals(capability)) {
                        // Choosing approximate location leaves FINE denied but grants COARSE.
                        allGranted = XXPermissions.isGrantedPermission(
                                activity, PermissionLists.getAccessCoarseLocationPermission());
                    } else if ("backgroundLocation".equals(capability)) {
                        allGranted = XXPermissions.isGrantedPermission(
                                activity, PermissionLists.getAccessBackgroundLocationPermission());
                    }
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
                    finish(activity);
                });
    }
}
