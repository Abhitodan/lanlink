# JNI entry points are looked up by name from native code.
-keep class com.lanlink.app.core.NativeBridge { *; }
-keepclasseswithmembernames class * { native <methods>; }
