package com.ryanheise.audioservice;

import android.content.Context;
import android.content.Intent;

public class MediaButtonReceiver extends androidx.media.session.MediaButtonReceiver {
    public static final String ACTION_NOTIFICATION_DELETE = "com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_DELETE";
    public static final String ACTION_NOTIFICATION_PLAY = "com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_PLAY";
    public static final String ACTION_NOTIFICATION_PAUSE = "com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_PAUSE";
    public static final String ACTION_NOTIFICATION_PREVIOUS = "com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_PREVIOUS";
    public static final String ACTION_NOTIFICATION_REWIND = "com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_REWIND";
    public static final String ACTION_NOTIFICATION_FAST_FORWARD = "com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_FAST_FORWARD";
    public static final String ACTION_NOTIFICATION_NEXT = "com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_NEXT";

    @Override
    public void onReceive(Context context, Intent intent) {
        if (intent != null && AudioService.instance != null) {
            String action = intent.getAction();
            if (ACTION_NOTIFICATION_DELETE.equals(action)) {
                AudioService.instance.handleDeleteNotification();
                return;
            }
            if (action != null && action.startsWith("com.ryanheise.audioservice.intent.action.ACTION_NOTIFICATION_")) {
                AudioService.instance.handleNotificationAction(action);
                return;
            }
        }
        super.onReceive(context, intent);
    }
}
