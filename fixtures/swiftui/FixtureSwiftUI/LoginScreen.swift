import SwiftUI

/// Username and password form. `login-status` reads `Signed out` until the
/// first submit, then `Welcome, <username>` when the password is `hunter2` and
/// `Invalid credentials` otherwise. Submitting dismisses the keyboard.
struct LoginScreen: View {
    static let validPassword = "hunter2"

    private enum Field {
        case username, password
    }

    @State private var username = ""
    @State private var password = ""
    @State private var status = "Signed out"
    @FocusState private var focusedField: Field?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            TextField("Username", text: $username)
                .focused($focusedField, equals: .username)
                .textFieldStyle(.roundedBorder)
                // Typed text must arrive verbatim, without capitalization or correction.
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityIdentifier("login-username")
            SecureField("Password", text: $password)
                .focused($focusedField, equals: .password)
                .textFieldStyle(.roundedBorder)
                .accessibilityIdentifier("login-password")
            Button("Sign in", action: submit)
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("login-submit")
            Text(verbatim: status)
                .accessibilityIdentifier("login-status")
            Spacer()
        }
        .padding()
    }

    private func submit() {
        focusedField = nil
        status = password == Self.validPassword ? "Welcome, \(username)" : "Invalid credentials"
    }
}
