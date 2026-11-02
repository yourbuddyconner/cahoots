import SwiftUI

/// The first screen: every circle this phone is a seat in.
struct CirclesView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if model.circles.isEmpty {
                    EmptyCircles()
                        .padding(.top, Space.s)
                } else {
                    LazyVStack(spacing: 0) {
                        ForEach(model.circles, id: \.id) { c in
                            NavigationLink(value: Route.chat(c.id)) {
                                CircleRow(circle: c, faces: model.faces[c.id] ?? [],
                                          others: model.others(circle: c.id, seats: (0..<c.members).filter { $0 != c.mySeat }),
                                          badge: AppModel.isCard(c) ? model.seatName(circle: c.id, seat: c.mySeat) : nil)
                            }
                            .buttonStyle(RowButtonStyle())
                            Hairline().padding(.leading, Space.l)
                        }
                    }
                    .padding(.top, Space.s)
                }
                if !model.hasCardCircle {
                    NavigationLink(value: Route.pairCard) { Text("Pair a card") }
                        .buttonStyle(TextButtonStyle(full: false))
                        .padding(.horizontal, Space.l)
                        .padding(.top, Space.s)
                        .accessibilityIdentifier("pair-card")
                }
                ChannelsSection()
                Text("Circles and history are saved on this phone. No account needed.")
                    .font(.footnote)
                    .foregroundStyle(Color.muted)
                    .padding(.horizontal, Space.l)
                    .padding(.top, Space.xl)
                    .padding(.bottom, Space.l)
            }
        }
        .paperBackground()
        .navigationTitle("Cahoots")
        .navigationBarTitleDisplayMode(.large)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                NavigationLink(value: Route.settings) {
                    Image(systemName: "gearshape")
                }
                .accessibilityLabel("Settings")
            }
        }
        .refreshable { _ = await model.pollAll() }
        // The two buttons are stacked, with or without circles: side by side, "Create a
        // circle" was cut off.
        .actionBar {
            create
            join
        }
    }

    private var create: some View {
        NavigationLink(value: Route.newCircle) { Text("Create a circle") }
            .buttonStyle(.filled)
            .accessibilityIdentifier("Create a circle")
    }

    private var join: some View {
        NavigationLink(value: Route.join) { Text("Join a circle") }
            .buttonStyle(.outlined)
            .accessibilityIdentifier("Join a circle")
    }
}

/// No circles yet: what Cahoots is for, in three steps.
private struct EmptyCircles: View {
    private let steps = [
        ("Create a circle", "Choose how many people will be in the circle, including you."),
        ("Compare the words", "Exchange codes with everyone joining. Check that every phone and badge shows the same six words in order."),
        ("Send messages", "Once everyone has kept the circle, you can send encrypted messages to each other."),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xxl) {
            ScreenHeader(title: "No circles yet",
                         detail: "A circle is a group of people who meet in person. Create one together or join one they have started. You don’t need an account or a phone number.",
                         style: .title)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(steps.enumerated()), id: \.offset) { i, step in
                    if i > 0 { Hairline() }
                    InstructionStep(number: i + 1, title: step.0, detail: step.1)
                    .padding(.vertical, Space.l)
                    .accessibilityElement(children: .combine)
                }
            }
        }
        .padding(.horizontal, Space.l)
    }
}

struct CircleRow: View {
    let circle: CircleSummary
    let faces: [Data]
    /// The other members' names, once any has one.
    var others: String?
    /// For a card circle, the name of its badge.
    var badge: String?
    @Environment(\.dynamicTypeSize) private var typeSize

    private var meta: String { Self.meta(circle, others: others, badge: badge) }

    /// The line under a circle's name. A card circle says so, and names its badge.
    static func meta(_ circle: CircleSummary, others: String?, badge: String?) -> String {
        var s = AppModel.isCard(circle) ? "Card circle on \(badge ?? "your badge"). " : ""
        s += others.map { "With \($0)." } ?? "\(spelled(Int(circle.members))) people."
        if !AppModel.isCard(circle) { s += " You are seat \(circle.mySeat + 1)." }
        if !circle.hasRoot && !AppModel.isCard(circle) { s += " Create a new circle together to chat." }
        if circle.needsTap { s += " Tap the card." }
        return s
    }

    /// A card circle's own seat is its badge's: the badge's face, from the card it published.
    private var badgeFace: Data? {
        AppModel.isCard(circle) && Int(circle.mySeat) < faces.count ? faces[Int(circle.mySeat)] : nil
    }

    var body: some View {
        HStack(alignment: .center, spacing: Space.m) {
            if let badgeFace {
                FaceView(pixels: badgeFace, size: 44, label: badge.map { "\($0)'s face" })
            }
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(circle.name)
                    .serifTitle(.title2)
                    .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
                Text(meta)
                    .font(.subheadline)
                    .foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
                if !faces.isEmpty {
                    FaceRow(faces: faces, size: 20).padding(.top, Space.xs)
                }
            }
            Spacer(minLength: Space.s)
            if circle.unread > 0 {
                Text("\(circle.unread)")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(Color.paper)
                    .padding(.horizontal, Space.s)
                    .frame(minWidth: 26, minHeight: 26)
                    .background(Color.brand, in: Capsule())
            }
            Image(systemName: "chevron.right")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Color.muted.opacity(0.6))
                .accessibilityHidden(true)
        }
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(circle.name). \(meta)\(circle.unread > 0 ? " \(circle.unread) unread." : "")")
    }
}
