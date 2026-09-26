import SwiftUI

/// A shape that rotates continuously while `anim-toggle` is on, so the screen
/// never settles. Rotation only changes pixels: the spinner's accessibility
/// frame and value stay constant while it turns.
struct AnimScreen: View {
    @State private var isAnimating = true
    @State private var isVisible = false

    var body: some View {
        VStack(spacing: 32) {
            // Rotates only while this tab is on screen, so other tabs can settle.
            Spinner(isRotating: isAnimating && isVisible)
                .frame(width: 160, height: 160)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("Spinner")
                .accessibilityValue(Text(verbatim: isAnimating ? "Spinning" : "Stopped"))
                .accessibilityIdentifier("anim-spinner")
            HStack(spacing: 12) {
                Text("Animate")
                    .accessibilityHidden(true)
                // Hiding the label shrinks the element to the switch itself, so a
                // tap at the element's center flips it.
                Toggle("Animate", isOn: $isAnimating)
                    .labelsHidden()
                    .accessibilityIdentifier("anim-toggle")
            }
            Spacer()
        }
        .padding()
        .onAppear { isVisible = true }
        .onDisappear { isVisible = false }
    }
}

private struct Spinner: View {
    let isRotating: Bool

    var body: some View {
        if isRotating {
            // The animation schedule lets the system pick the update interval.
            TimelineView(.animation) { context in
                SpinnerCanvas(degrees: Self.degrees(at: context.date))
            }
        } else {
            SpinnerCanvas(degrees: 0)
        }
    }

    /// One full turn per second.
    static func degrees(at date: Date) -> Double {
        date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1) * 360
    }
}

/// Draws a three-quarter arc starting at `degrees`. The rotation happens inside
/// the canvas, so the view's frame, and with it the accessibility frame, never
/// moves. A rotation effect on a child view would move the accessibility frame
/// as the shape turns.
private struct SpinnerCanvas: View {
    let degrees: Double

    var body: some View {
        Canvas { context, size in
            let lineWidth: CGFloat = 16
            var arc = Path()
            arc.addArc(
                center: CGPoint(x: size.width / 2, y: size.height / 2),
                radius: min(size.width, size.height) / 2 - lineWidth,
                startAngle: .degrees(degrees),
                endAngle: .degrees(degrees + 270),
                clockwise: false
            )
            context.stroke(
                arc,
                with: .color(.accentColor),
                style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)
            )
        }
    }
}
