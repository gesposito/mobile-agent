import CoreLocation
import SwiftUI

/// Two alerts with different owners: `system-permission` asks for location
/// access, which system UI presents outside the app, and `system-alert` shows
/// an alert the app presents itself.
struct SystemScreen: View {
    @StateObject private var location = LocationPermission()
    @State private var isShowingAlert = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Button("Request location permission") { location.request() }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("system-permission")
            Text(verbatim: location.statusText)
                .accessibilityIdentifier("system-permission-status")
            Button("Show in-app alert") { isShowingAlert = true }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("system-alert")
            Spacer()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding()
        .alert("In-app alert", isPresented: $isShowingAlert) {
            Button("OK") {}
                .accessibilityIdentifier("inapp-alert-ok")
        } message: {
            Text("This alert is presented by the app.")
        }
    }
}

/// Owns the location manager that requests when-in-use authorization and
/// publishes the current status. The status is read from the system, so it is
/// `unknown` only while authorization is not determined, which is also the only
/// state in which the system shows its alert.
@MainActor
final class LocationPermission: NSObject, ObservableObject, CLLocationManagerDelegate {
    @Published private(set) var status: CLAuthorizationStatus
    private let manager: CLLocationManager

    override init() {
        let manager = CLLocationManager()
        self.manager = manager
        status = manager.authorizationStatus
        super.init()
        manager.delegate = self
    }

    var statusText: String {
        switch status {
        case .authorizedWhenInUse, .authorizedAlways: "Permission: granted"
        case .denied, .restricted: "Permission: denied"
        case .notDetermined: "Permission: unknown"
        @unknown default: "Permission: unknown"
        }
    }

    func request() {
        manager.requestWhenInUseAuthorization()
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let status = manager.authorizationStatus
        Task { @MainActor in
            self.status = status
        }
    }
}
