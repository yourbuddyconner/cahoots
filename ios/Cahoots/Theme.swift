import SwiftUI

// The app's look: six colours, a type scale, and the few controls drawn by hand.
// ios/DESIGN.md has the reasons.

extension Color {
    /// Every screen's ground.
    static let paper = Color("Paper")
    /// Text and the outlines of controls.
    static let ink = Color("Ink")
    /// The one accent.
    static let brand = Color("Blue")
    /// Hairlines between rows and around fields.
    static let line = Color("Line")
    /// Secondary text.
    static let muted = Color("Muted")
    /// Only behind a face, as on the badge's e-paper screen.
    static let cream = Color("Cream")
}

enum Space {
    static let xs: CGFloat = 4
    static let s: CGFloat = 8
    static let m: CGFloat = 12
    static let l: CGFloat = 16
    static let xl: CGFloat = 24
    static let xxl: CGFloat = 32
}

enum Radius {
    static let control: CGFloat = 12
}

extension View {
    /// A screen's title in the system serif, as the site's Georgia headlines.
    func serifTitle(_ style: Font.TextStyle = .largeTitle) -> some View {
        font(.system(style, design: .serif)).foregroundStyle(Color.ink)
    }

    /// Paper behind the whole screen, under the bars too.
    func paperBackground() -> some View {
        background(Color.paper.ignoresSafeArea())
    }

    /// A bar of actions pinned to the bottom. On iOS 26 it floats over the content and the
    /// scroll edge softens under it. Before, it sits on paper with a hairline above.
    /// `shown` false leaves no bar at all: an empty bar still takes touches above the keyboard.
    @ViewBuilder
    func actionBar<Bar: View>(shown: Bool = true, @ViewBuilder _ bar: () -> Bar) -> some View {
        if !shown {
            self
        } else if #available(iOS 26, *) {
            safeAreaBar(edge: .bottom) {
                VStack(spacing: Space.s) { bar() }
                    .padding(.horizontal, Space.l)
                    .padding(.top, Space.s)
                    .padding(.bottom, Space.s)
            }
        } else {
            safeAreaInset(edge: .bottom) {
                VStack(spacing: Space.s) { bar() }
                    .padding(.horizontal, Space.l)
                    .padding(.vertical, Space.m)
                    .frame(maxWidth: .infinity)
                    .background(alignment: .top) {
                        Color.paper.ignoresSafeArea()
                            .overlay(alignment: .top) { Hairline() }
                    }
            }
        }
    }
}

/// A one-pixel line in Line.
struct Hairline: View {
    @Environment(\.displayScale) private var scale
    var body: some View {
        Rectangle().fill(Color.line).frame(height: 1 / scale)
            .accessibilityHidden(true)
    }
}

// MARK: - Buttons

/// The one main action on a screen: blue, full width.
struct FilledButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundStyle(Color.paper)
            .multilineTextAlignment(.center)
            .padding(.vertical, Space.s)
            .frame(maxWidth: .infinity, minHeight: 52)
            .padding(.horizontal, Space.l)
            .background(Color.brand, in: RoundedRectangle(cornerRadius: Radius.control))
            .opacity(enabled ? (configuration.isPressed ? 0.82 : 1) : 0.4)
            .contentShape(RoundedRectangle(cornerRadius: Radius.control))
    }
}

/// The second action: ink in a hairline outline.
struct OutlinedButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.medium))
            .foregroundStyle(Color.ink)
            .multilineTextAlignment(.center)
            .padding(.vertical, Space.s)
            .frame(maxWidth: .infinity, minHeight: 52)
            .padding(.horizontal, Space.l)
            .background(configuration.isPressed ? Color.line.opacity(0.6) : Color.paper,
                        in: RoundedRectangle(cornerRadius: Radius.control))
            .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.ink.opacity(0.22)))
            .opacity(enabled ? 1 : 0.4)
            .contentShape(RoundedRectangle(cornerRadius: Radius.control))
    }
}

/// A quiet action: blue words, at least 44 points tall.
struct TextButtonStyle: ButtonStyle {
    var full = true
    var color = Color.brand
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.medium))
            .foregroundStyle(color)
            .frame(maxWidth: full ? .infinity : nil, minHeight: 44)
            .contentShape(Rectangle())
            .opacity(configuration.isPressed ? 0.5 : 1)
    }
}

extension ButtonStyle where Self == FilledButtonStyle {
    static var filled: FilledButtonStyle { FilledButtonStyle() }
}

extension ButtonStyle where Self == OutlinedButtonStyle {
    static var outlined: OutlinedButtonStyle { OutlinedButtonStyle() }
}

extension ButtonStyle where Self == TextButtonStyle {
    static var text: TextButtonStyle { TextButtonStyle() }
}

/// A small outlined button beside a field, as tall as the field.
struct CompactButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.medium))
            .foregroundStyle(Color.brand)
            .padding(.horizontal, Space.l)
            .frame(minWidth: 64, minHeight: 48)
            .background(configuration.isPressed ? Color.line.opacity(0.6) : Color.paper,
                        in: RoundedRectangle(cornerRadius: Radius.control))
            .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line))
            .contentShape(RoundedRectangle(cornerRadius: Radius.control))
            .opacity(enabled ? 1 : 0.4)
    }
}

/// A row that dims while pressed, for whole rows that navigate.
struct RowButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .contentShape(Rectangle())
            .background(configuration.isPressed ? Color.line.opacity(0.5) : Color.clear)
    }
}

/// A button's label with a spinner in place of the words while it works.
struct Working: View {
    let busy: Bool
    let title: String

    var body: some View {
        ZStack {
            Text(title).opacity(busy ? 0 : 1)
            if busy { ProgressView().tint(Color.paper) }
        }
    }
}

// MARK: - Sections and rows

/// A short, numbered instruction that stays readable at larger text sizes.
struct InstructionStep: View {
    let number: Int
    let title: String
    let detail: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            Text("\(number)")
                .font(.subheadline.weight(.semibold).monospacedDigit())
                .foregroundStyle(Color.brand)
                .frame(minWidth: 20)
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(title).font(.headline).foregroundStyle(Color.ink)
                Text(detail).font(.subheadline).foregroundStyle(Color.muted)
            }
            .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
    }
}

/// A titled group of rows split by hairlines, on paper: the app's grouped list.
struct PaperSection<Content: View>: View {
    var title: String?
    var footer: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let title {
                Text(title)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Color.muted)
                    .padding(.bottom, Space.s)
                    .accessibilityAddTraits(.isHeader)
            }
            Hairline()
            content
            Hairline()
            if let footer {
                Text(footer)
                    .font(.footnote)
                    .foregroundStyle(Color.muted)
                    .padding(.top, Space.s)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A choice in words: a title, a line under it, and a blue check when chosen.
struct OptionRow: View {
    let title: String
    var detail: String?
    let chosen: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.body).foregroundStyle(Color.ink)
                    if let detail {
                        Text(detail).font(.subheadline).foregroundStyle(Color.muted)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: Space.s)
                Image(systemName: "checkmark")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(Color.brand)
                    .opacity(chosen ? 1 : 0)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, Space.m)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        }
        .buttonStyle(RowButtonStyle())
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }
}

/// A screen's opening: a serif title and, under it, one or two sentences.
struct ScreenHeader: View {
    let title: String
    var detail: String?
    var style: Font.TextStyle = .largeTitle

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text(title)
                .serifTitle(style)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            if let detail {
                Text(detail)
                    .font(.body)
                    .foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A field with a hairline border, the app's text field.
struct FieldStyle: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(.horizontal, Space.m)
            .padding(.vertical, Space.m)
            .frame(minHeight: 48)
            .background(Color.paper, in: RoundedRectangle(cornerRadius: Radius.control))
            .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line))
            .foregroundStyle(Color.ink)
    }
}

extension View {
    func paperField() -> some View { modifier(FieldStyle()) }
}

/// Numbers in words for meta sentences: "Three people."
func spelled(_ n: Int) -> String {
    let words = ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten"]
    return n < words.count ? words[n] : "\(n)"
}
