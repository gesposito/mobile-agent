// Passes the FIXTURE_INITIAL_TAB launch option (fixtures/CONTRACT.md, Launch options) to
// JavaScript as the root component's `initialTab` prop. Prebuild regenerates ios/ and android/,
// so the native edits live in this config plugin rather than in the generated files. Each edit
// throws when the code it anchors on is missing, so a template change fails prebuild instead
// of silently dropping the option. On iOS the code is React Native's start, which
// withSceneLifecycle.js moves into the scene delegate: the anchor is the call, wherever it is.

// iOS: `xcrun simctl launch` forwards SIMCTL_CHILD_FIXTURE_INITIAL_TAB to the app's
// environment as FIXTURE_INITIAL_TAB.
function addInitialTabToAppDelegate(contents) {
  if (contents.includes('FIXTURE_INITIAL_TAB')) {
    return contents;
  }
  const anchor = /(\n([ \t]*)in: window,\n)([ \t]*launchOptions: launchOptions\))/;
  if (!anchor.test(contents)) {
    throw new Error('withInitialTab: startReactNative call not found in AppDelegate.swift');
  }
  return contents.replace(
    anchor,
    '$1$2initialProperties: ["initialTab": ProcessInfo.processInfo.environment["FIXTURE_INITIAL_TAB"] ?? ""],\n$3',
  );
}

// Android: the string extra `initial_tab` on the launch intent.
function addInitialTabToMainActivity(contents) {
  if (contents.includes('"initial_tab"')) {
    return contents;
  }
  const anchor = /(object : DefaultReactActivityDelegate\([^)]*\)\s*)\{\s*\}/;
  if (!anchor.test(contents)) {
    throw new Error('withInitialTab: DefaultReactActivityDelegate not found in MainActivity.kt');
  }
  if (!contents.includes('import android.os.Bundle')) {
    throw new Error('withInitialTab: MainActivity.kt no longer imports android.os.Bundle');
  }
  return contents.replace(
    anchor,
    '$1{\n' +
      '              override fun getLaunchOptions(): Bundle? =\n' +
      '                  (super.getLaunchOptions() ?: Bundle()).apply {\n' +
      '                    putString("initialTab", this@MainActivity.intent?.getStringExtra("initial_tab") ?: "")\n' +
      '                  }\n' +
      '          }',
  );
}

function withInitialTab(config) {
  // Required here, so that the edits above load without the project's packages (test/).
  const { withAppDelegate, withMainActivity } = require('expo/config-plugins');
  config = withAppDelegate(config, (mod) => {
    if (mod.modResults.language !== 'swift') {
      throw new Error(`withInitialTab: expected a Swift AppDelegate, got ${mod.modResults.language}`);
    }
    mod.modResults.contents = addInitialTabToAppDelegate(mod.modResults.contents);
    return mod;
  });
  return withMainActivity(config, (mod) => {
    if (mod.modResults.language !== 'kt') {
      throw new Error(`withInitialTab: expected a Kotlin MainActivity, got ${mod.modResults.language}`);
    }
    mod.modResults.contents = addInitialTabToMainActivity(mod.modResults.contents);
    return mod;
  });
}

module.exports = withInitialTab;
module.exports.addInitialTabToAppDelegate = addInitialTabToAppDelegate;
module.exports.addInitialTabToMainActivity = addInitialTabToMainActivity;
