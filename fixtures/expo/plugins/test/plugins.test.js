// The local config plugins' edits of the generated iOS project, on the AppDelegate.swift of Expo
// SDK 57's template (AppDelegate.swift beside this file), with Node's own test runner and no
// package: `node --test fixtures/expo/plugins/test/plugins.test.js`. fixtures/test.sh runs it.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const { addInitialTabToAppDelegate } = require('../withInitialTab');
const { adoptSceneLifecycle, declareScene } = require('../withSceneLifecycle');

const template = fs.readFileSync(path.join(__dirname, 'AppDelegate.swift'), 'utf8');
const INITIAL_TAB =
  'initialProperties: ["initialTab": ProcessInfo.processInfo.environment["FIXTURE_INITIAL_TAB"] ?? ""],';

// The part of the file from the declaration of `name` to the next top-level declaration.
function classBody(contents, name) {
  const start = contents.indexOf(`class ${name}`);
  assert.notEqual(start, -1, `no class ${name} in:\n${contents}`);
  const next = contents.slice(start + 1).search(/\n(@main\n)?class /);
  return next === -1 ? contents.slice(start) : contents.slice(start, start + 1 + next);
}

test('the app delegate provides the factory and leaves the window and the start to a scene delegate', () => {
  const edited = adoptSceneLifecycle(template);
  const app = classBody(edited, 'AppDelegate');
  assert.match(app, /^class AppDelegate: ExpoAppDelegate, ExpoReactNativeFactoryProvider \{/);
  assert.doesNotMatch(app, /UIWindow\(/);
  assert.doesNotMatch(app, /startReactNative/);
  assert.match(app, /reactNativeFactory = factory/);
  assert.match(app, /return super\.application\(application, didFinishLaunchingWithOptions: launchOptions\)/);

  const scene = classBody(edited, 'SceneDelegate');
  assert.match(scene, /^class SceneDelegate: ExpoAppSceneDelegate \{/);
  assert.match(scene, /override func scene\(\n\s*_ scene: UIScene,\n\s*willConnectTo session: UISceneSession,/);
  assert.match(scene, /let window = UIWindow\(windowScene: windowScene\)/);
  assert.match(scene, /appDelegate\.window = window/);
  assert.match(
    scene,
    /factory\.startReactNative\(\n\s*withModuleName: "main",\n\s*in: window,\n\s*launchOptions: launchOptions\)/,
  );
});

test('the initial tab reaches the scene delegate whichever plugin edits the file first', () => {
  for (const edited of [
    addInitialTabToAppDelegate(adoptSceneLifecycle(template)),
    adoptSceneLifecycle(addInitialTabToAppDelegate(template)),
  ]) {
    assert.doesNotMatch(classBody(edited, 'AppDelegate'), /startReactNative|initialTab/);
    const scene = classBody(edited, 'SceneDelegate');
    assert.ok(scene.includes(INITIAL_TAB), scene);
    assert.match(scene, /in: window,\n\s*initialProperties: .*\n\s*launchOptions: launchOptions\)/);
  }
});

test('a second run changes nothing', () => {
  const once = adoptSceneLifecycle(template);
  assert.equal(adoptSceneLifecycle(once), once);
});

test('a template without the start the plugin moves fails the prebuild', () => {
  assert.throws(
    () => adoptSceneLifecycle(template.replace('window = UIWindow(frame: UIScreen.main.bounds)', '')),
    /withSceneLifecycle: /,
  );
});

test('Info.plist declares one scene, played by the scene delegate', () => {
  const plist = declareScene({ CFBundleName: 'FixtureExpo' });
  assert.equal(plist.CFBundleName, 'FixtureExpo');
  assert.deepEqual(plist.UIApplicationSceneManifest, {
    UIApplicationSupportsMultipleScenes: false,
    UISceneConfigurations: {
      UIWindowSceneSessionRoleApplication: [
        {
          UISceneConfigurationName: 'Default Configuration',
          UISceneDelegateClassName: '$(PRODUCT_MODULE_NAME).SceneDelegate',
        },
      ],
    },
  });
});
