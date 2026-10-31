// Moves the iOS app to UIKit's scene life cycle. iOS 27 stops an app built with the iOS 27 SDK at
// launch unless it adopts it (Apple's TN3187): UIKit traps in
// _UIApplicationEvaluateRuntimeIssueForNoSceneLifecycleAdoption, SIGTRAP, before the first frame.
// Expo SDK 57's template keeps the application life cycle, the app delegate making the window and
// starting React Native in it; SDK 58's adopts scenes.
//
// Info.plist declares one scene; the app delegate provides its React Native factory
// (ExpoReactNativeFactoryProvider) and no longer starts React Native; a scene delegate, a subclass
// of Expo's ExpoAppSceneDelegate, which forwards the scene's life-cycle events to the app delegate,
// makes the window from the scene UIKit connects and starts React Native in it with the template's
// call, as the other plugins left it: withInitialTab.js adds the initial properties to that call
// before or after this plugin moves it. Expo's own opt-in, expo-build-properties'
// ios.enableSceneSupport, plays ExpoAppSceneDelegate itself, whose start passes no initial
// properties. iOS 26 runs the scene life cycle too.
//
// Each edit throws when the code it anchors on is missing, so a template change fails prebuild.

// The template's start: the window and the call, in the app delegate's
// application(_:didFinishLaunchingWithOptions:).
const START =
  /\n#if os\(iOS\) \|\| os\(tvOS\)\n[ \t]*window = UIWindow\(frame: UIScreen\.main\.bounds\)\n([ \t]*factory\.startReactNative\([\s\S]*?launchOptions: launchOptions\))\n#endif\n/;
const APP_DELEGATE = 'class AppDelegate: ExpoAppDelegate {';
const SCENE_DELEGATE = 'class SceneDelegate: ExpoAppSceneDelegate {';

function sceneDelegate(start) {
  return `
// The window of the scene UIKit connects, where React Native starts as the template started it in
// the app delegate (plugins/withSceneLifecycle.js).
${SCENE_DELEGATE}
  override func scene(
    _ scene: UIScene,
    willConnectTo session: UISceneSession,
    options connectionOptions: UIScene.ConnectionOptions
  ) {
    guard let windowScene = scene as? UIWindowScene,
      let appDelegate = UIApplication.shared.delegate as? AppDelegate,
      let factory = appDelegate.reactNativeFactory
    else {
      return
    }
    let window = UIWindow(windowScene: windowScene)
    self.window = window
    appDelegate.window = window
    let launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
${start}
  }
}
`;
}

function adoptSceneLifecycle(contents) {
  if (contents.includes(SCENE_DELEGATE)) {
    return contents;
  }
  const start = contents.match(START);
  if (!start) {
    throw new Error(
      'withSceneLifecycle: the window and the startReactNative call not found in AppDelegate.swift',
    );
  }
  if (!contents.includes(APP_DELEGATE)) {
    throw new Error(`withSceneLifecycle: "${APP_DELEGATE}" not found in AppDelegate.swift`);
  }
  return (
    contents
      .replace(START, '')
      .replace(APP_DELEGATE, 'class AppDelegate: ExpoAppDelegate, ExpoReactNativeFactoryProvider {') +
    sceneDelegate(start[1])
  );
}

function declareScene(infoPlist) {
  return {
    ...infoPlist,
    UIApplicationSceneManifest: {
      UIApplicationSupportsMultipleScenes: false,
      UISceneConfigurations: {
        UIWindowSceneSessionRoleApplication: [
          {
            UISceneConfigurationName: 'Default Configuration',
            UISceneDelegateClassName: '$(PRODUCT_MODULE_NAME).SceneDelegate',
          },
        ],
      },
    },
  };
}

function withSceneLifecycle(config) {
  // Required here, so that the edits above load without the project's packages (test/).
  const { withAppDelegate, withInfoPlist } = require('expo/config-plugins');
  config = withAppDelegate(config, (mod) => {
    if (mod.modResults.language !== 'swift') {
      throw new Error(
        `withSceneLifecycle: expected a Swift AppDelegate, got ${mod.modResults.language}`,
      );
    }
    mod.modResults.contents = adoptSceneLifecycle(mod.modResults.contents);
    return mod;
  });
  return withInfoPlist(config, (mod) => {
    mod.modResults = declareScene(mod.modResults);
    return mod;
  });
}

module.exports = withSceneLifecycle;
module.exports.adoptSceneLifecycle = adoptSceneLifecycle;
module.exports.declareScene = declareScene;
