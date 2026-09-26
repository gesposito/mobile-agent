import SwiftUI
import UIKit

enum FixtureTab: String, CaseIterable {
    case login, list, big, anim, system

    /// The tab named by the `FIXTURE_INITIAL_TAB` launch environment variable,
    /// or `login` when the variable is unset or names no tab.
    static var initial: FixtureTab {
        ProcessInfo.processInfo.environment["FIXTURE_INITIAL_TAB"].flatMap(FixtureTab.init(rawValue:)) ?? .login
    }

    var title: String {
        switch self {
        case .login: "Login"
        case .list: "List"
        case .big: "Big"
        case .anim: "Anim"
        case .system: "System"
        }
    }

    var systemImage: String {
        switch self {
        case .login: "person.crop.circle"
        case .list: "list.bullet"
        case .big: "square.grid.3x3"
        case .anim: "arrow.triangle.2.circlepath"
        case .system: "exclamationmark.bubble"
        }
    }

    @MainActor @ViewBuilder var screen: some View {
        switch self {
        case .login: LoginScreen()
        case .list: ListScreen()
        case .big: BigScreen()
        case .anim: AnimScreen()
        case .system: SystemScreen()
        }
    }
}

/// Bottom tab bar hosting every screen: a UIKit tab bar controller with one
/// hosting controller per SwiftUI screen. Each tab bar item gets its
/// `tab-<name>` identifier when it is created, so the identifiers are in the
/// first accessibility snapshot. A SwiftUI TabView attaches them to the tab bar
/// only in a later update, which on iOS 26.5 came up to a second after launch or
/// not at all.
///
/// Only the selected tab's view is in the window, so the other screens are
/// absent from the accessibility tree, and each hosting controller keeps its
/// screen's state while another tab is selected.
struct RootView: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> UITabBarController {
        let controller = UITabBarController()
        controller.viewControllers = FixtureTab.allCases.map { tab in
            // Pinned so the host's text size setting cannot change the layout.
            let screen = UIHostingController(rootView: tab.screen.dynamicTypeSize(.large))
            screen.tabBarItem = UITabBarItem(
                title: tab.title,
                image: UIImage(systemName: tab.systemImage),
                selectedImage: nil
            )
            screen.tabBarItem.accessibilityIdentifier = "tab-\(tab.rawValue)"
            return screen
        }
        controller.selectedIndex = FixtureTab.allCases.firstIndex(of: .initial) ?? 0
        return controller
    }

    func updateUIViewController(_ controller: UITabBarController, context: Context) {}
}
