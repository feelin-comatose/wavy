# NewPipeExtractor and Rhino (YouTube signature deciphering) work through reflection
-keep class org.schabi.newpipe.extractor.** { *; }
-keep class org.mozilla.javascript.** { *; }
-keep class org.mozilla.classfile.ClassFileWriter
-dontwarn org.mozilla.javascript.**
-dontwarn javax.script.**
-dontwarn java.beans.**
-dontwarn jdk.dynalink.**
-dontwarn org.jsoup.**
-dontwarn com.google.re2j.**
-dontwarn org.slf4j.**
# library models are serialized with kotlinx.serialization
-keepclassmembers class app.wavy.android.data.** { *; }
# protobuf-lite (NewPipe's YouTube requests) reads message fields through reflection — names must not change
-keep class * extends com.google.protobuf.GeneratedMessageLite { *; }
-dontwarn com.google.protobuf.**
# No class renaming: R8 only strips unused code. That way the reflection-based libraries
# (NewPipe, Rhino, protobuf, kotlinx.serialization) are guaranteed to find everything by name.
-dontobfuscate
-keep class app.wavy.android.data.** { *; }
