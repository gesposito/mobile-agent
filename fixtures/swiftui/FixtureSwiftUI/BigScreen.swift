import SwiftUI

/// Exactly 500 actionable cells in a LazyVGrid. A lazy grid only creates the
/// cells it renders, so the grid is sized to fit the screen without scrolling:
/// every cell is rendered, and therefore in the accessibility tree, at once.
/// The `big-count` header is not one of the 500.
struct BigScreen: View {
    static let cellCount = 500
    static let columnCount = 20
    static let rowCount = cellCount / columnCount

    var body: some View {
        VStack(spacing: 8) {
            Text(verbatim: "\(Self.cellCount) elements")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("big-count")
            GeometryReader { proxy in
                LazyVGrid(
                    columns: Array(repeating: GridItem(.flexible(), spacing: 0), count: Self.columnCount),
                    spacing: 0
                ) {
                    ForEach(1...Self.cellCount, id: \.self) { n in
                        BigCell(n: n)
                            .frame(height: proxy.size.height / CGFloat(Self.rowCount))
                    }
                }
            }
        }
        .padding(.horizontal, 8)
        .padding(.top, 8)
    }
}

/// One grid cell: a button labelled `Cell <n>`. A tap only highlights it while
/// pressed, so the tree is the same before and after any tap.
private struct BigCell: View {
    let n: Int

    var body: some View {
        Button {} label: {
            Text(verbatim: String(n))
                .font(.system(size: 9, weight: .medium, design: .monospaced))
                .minimumScaleFactor(0.5)
                .lineLimit(1)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(
                    RoundedRectangle(cornerRadius: 3)
                        .fill(Color(uiColor: .systemGray5))
                        .padding(1)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(verbatim: "Cell \(n)"))
        .accessibilityIdentifier("cell-\(n)")
    }
}
