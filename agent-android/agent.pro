# R8 rules for the agent (task 4.10), read by build.sh.
#
# app_process loads the entry point by name and calls main through
# reflection, so R8 must keep that class and method as they are. Every other
# reflective call in the agent targets framework members (UiAutomation,
# InputManagerGlobal, AccessibilityInteractionClient and the like), which
# live in the library, android.jar, not in the program, so R8 neither
# renames nor removes them and they need no rule.
-keep class dev.mobileagent.agent.Main {
    public static void main(java.lang.String[]);
}

# Readable class and method names in stack traces and in the messages of
# `internal` errors, which name the exception class.
-dontobfuscate
-keepattributes SourceFile,LineNumberTable
