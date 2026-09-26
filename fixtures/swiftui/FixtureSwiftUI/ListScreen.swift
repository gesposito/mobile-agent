import SwiftUI

/// 200 rows in a List inside a navigation stack. Only rows on screen exist in
/// the accessibility tree; the rest appear as the list scrolls. Tapping
/// `row-<n>` pushes a detail screen. The navigation bar stays hidden, so the
/// only way back is the detail screen's own `detail-back` button.
struct ListScreen: View {
    static let rowCount = 200

    var body: some View {
        NavigationStack {
            List(1...Self.rowCount, id: \.self) { n in
                NavigationLink(value: n) {
                    Text(verbatim: "Row \(n)")
                }
                .accessibilityIdentifier("row-\(n)")
            }
            .listStyle(.plain)
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(for: Int.self) { n in
                RowDetailScreen(n: n)
            }
        }
    }
}

/// Detail for one row: a `detail-<n>` label reading `Detail <n>` and a
/// `detail-back` button that pops back to the list.
struct RowDetailScreen: View {
    let n: Int

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 24) {
            Text(verbatim: "Detail \(n)")
                .font(.largeTitle.bold())
                .accessibilityIdentifier("detail-\(n)")
            Button("Back") { dismiss() }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("detail-back")
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbar(.hidden, for: .navigationBar)
    }
}
