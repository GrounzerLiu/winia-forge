package com.winia.permissions;

import android.accessibilityservice.AccessibilityService;
import android.app.Activity;
import android.app.admin.DeviceAdminReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Build;
import android.os.Looper;
import android.provider.Settings;
import android.service.notification.NotificationListenerService;

import com.hjq.permissions.XXPermissions;
import com.hjq.permissions.permission.PermissionLists;
import com.hjq.permissions.permission.base.IPermission;
import com.hjq.permissions.tools.PermissionSettingPage;

import java.util.ArrayDeque;
import java.util.HashSet;
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
    private static long ACTIVE_ID;
    private static Activity ACTIVE_ACTIVITY;
    private static final String PREFS = "winia_permissions";

    private static final class RequestTask {
        final Activity activity;
        final long requestId;
        final Runnable runnable;

        RequestTask(Activity activity, long requestId, Runnable runnable) {
            this.activity = activity;
            this.requestId = requestId;
            this.runnable = runnable;
        }
    }

    private static native void nativeOnResult(long requestId, boolean granted, boolean doNotAskAgain);

    private WiniaPermissions() {}

    public static void requestCapability(Activity activity, long requestId, String capability) {
        markRequested(activity, capability);
        IPermission[] permissions = permissionsFor(capability);
        enqueue(activity, requestId, () -> request(activity, requestId, capability, permissions));
    }

    public static void requestService(Activity activity, long requestId, String kind, String className) {
        IPermission permission = servicePermission(kind, className);
        enqueue(activity, requestId, () -> request(activity, requestId, "", permission));
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

    public static boolean hasDeclaredPermission(Activity activity, String capability) {
        if ("notifications".equals(capability) && Build.VERSION.SDK_INT < 33) {
            return true;
        }
        try {
            PackageInfo info = activity.getPackageManager().getPackageInfo(
                    activity.getPackageName(), PackageManager.GET_PERMISSIONS);
            HashSet<String> declared = new HashSet<>();
            if (info.requestedPermissions != null) {
                for (String name : info.requestedPermissions) {
                    declared.add(name);
                }
            }
            if ("location".equals(capability)) {
                // permissionsFor("location") requests both; keep the preflight aligned.
                return declared.contains("android.permission.ACCESS_FINE_LOCATION")
                        && declared.contains("android.permission.ACCESS_COARSE_LOCATION");
            }
            for (IPermission permission : permissionsFor(capability)) {
                if (!declared.contains(permission.getPermissionName())) {
                    return false;
                }
            }
            return true;
        } catch (PackageManager.NameNotFoundException error) {
            return false;
        }
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
        activity.runOnUiThread(() -> {
            List<Intent> intents = PermissionSettingPage.getCommonPermissionSettingIntent(activity, permissions);
            Intent intent = intents.isEmpty()
                    ? PermissionSettingPage.getApplicationDetailsSettingsIntent(activity, permissions)
                    : intents.get(0);
            activity.startActivity(intent);
        });
    }

    public static void openSettingsForService(Activity activity, String kind, String className) {
        IPermission permission = servicePermission(kind, className);
        activity.runOnUiThread(() -> {
            List<Intent> intents = PermissionSettingPage.getCommonPermissionSettingIntent(activity, permission);
            Intent intent = intents.isEmpty()
                    ? PermissionSettingPage.getApplicationDetailsSettingsIntent(activity, permission)
                    : intents.get(0);
            activity.startActivity(intent);
        });
    }

    public static void openSettings(Activity activity) {
        activity.runOnUiThread(() -> {
            Intent intent = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS);
            intent.setData(Uri.parse("package:" + activity.getPackageName()));
            activity.startActivity(intent);
        });
    }

    public static int takeResult(long requestId) {
        synchronized (QUEUE_LOCK) {
            if (CANCELLED.contains(requestId)) {
                return -1;
            }
            Integer value = RESULTS.remove(requestId);
            return value == null ? -1 : value;
        }
    }

    public static boolean cancelAllIfDestroyed(Activity activity) {
        if (activity.isFinishing() || activity.isDestroyed()) {
            cancelAll();
            return true;
        }
        return false;
    }

    public static void cancelAll() {
        synchronized (QUEUE_LOCK) {
            QUEUE.clear();
            ACTIVE = false;
            ACTIVE_ID = 0;
            ACTIVE_ACTIVITY = null;
            CANCELLED.addAll(PENDING);
            PENDING.clear();
            RESULTS.clear();
        }
    }

    public static void clearResult(long requestId) {
        synchronized (QUEUE_LOCK) {
            if (QUEUE.removeIf(task -> task.requestId == requestId)) {
                return;
            }
            if (ACTIVE_ID == requestId) {
                // Do not start the next request yet: the system dialog for this request may still
                // be visible. Its callback owns queue advancement.
                CANCELLED.add(requestId);
            }
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

    private static void enqueue(Activity activity, long requestId, Runnable task) {
        synchronized (QUEUE_LOCK) {
            if (ACTIVE) {
                QUEUE.add(new RequestTask(activity, requestId, task));
                return;
            }
            ACTIVE = true;
            ACTIVE_ID = requestId;
            ACTIVE_ACTIVITY = activity;
        }
        activity.runOnUiThread(task);
    }

    private static void finish(Activity activity, long requestId) {
        RequestTask next;
        synchronized (QUEUE_LOCK) {
            if (ACTIVE_ID != requestId) {
                return;
            }
            ACTIVE_ID = 0;
            ACTIVE_ACTIVITY = null;
            next = QUEUE.poll();
            if (next == null) {
                ACTIVE = false;
                return;
            }
            ACTIVE_ID = next.requestId;
            ACTIVE_ACTIVITY = next.activity;
        }
        next.activity.runOnUiThread(next.runnable);
    }

    private static void request(Activity activity, long requestId, String capability, IPermission... permissions) {
        synchronized (QUEUE_LOCK) {
            if (ACTIVE_ID != requestId || CANCELLED.remove(requestId)) {
                finish(activity, requestId);
                return;
            }
            PENDING.add(requestId);
        }
        try {
            XXPermissions.with(activity)
                    .permissions(permissions)
                    .request((granted, denied) -> {
                    int value = 0;
                    boolean cancelled;
                    synchronized (QUEUE_LOCK) {
                        PENDING.remove(requestId);
                        cancelled = CANCELLED.remove(requestId);
                        if (!cancelled) {
                            boolean allGranted = denied.isEmpty();
                            if ("location".equals(capability)) {
                                // Choosing approximate location leaves FINE denied but grants COARSE.
                                allGranted = XXPermissions.isGrantedPermission(
                                        activity, PermissionLists.getAccessCoarseLocationPermission());
                            } else if ("backgroundLocation".equals(capability)) {
                                allGranted = XXPermissions.isGrantedPermission(
                                        activity, PermissionLists.getAccessBackgroundLocationPermission());
                            }
                            if (allGranted) {
                                value = 1;
                            } else if (XXPermissions.isDoNotAskAgainPermissions(activity, denied)) {
                                value = 2;
                            }
                            RESULTS.put(requestId, value);
                        }
                    }
                    if (cancelled) {
                        finish(activity, requestId);
                        return;
                    }
                    nativeOnResult(requestId, value == 1, value == 2);
                    finish(activity, requestId);
                    });
        } catch (RuntimeException error) {
            synchronized (QUEUE_LOCK) {
                PENDING.remove(requestId);
                CANCELLED.remove(requestId);
            }
            finish(activity, requestId);
            throw error;
        }
    }
}
