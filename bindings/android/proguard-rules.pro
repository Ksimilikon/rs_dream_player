# The core constructs these classes and calls the listener from native code,
# by name — R8 must neither remove nor rename them.
-keep class dev.dreamplayer.core.** { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}
