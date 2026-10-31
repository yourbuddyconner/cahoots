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
                                CircleRow(circle: c, faces: model.faces[c.id] ?? [])
                            }
                            .buttonStyle(RowButtonStyle())
                            Hairline().padding(.leading, Space.l)
                        }
                    }
                    .padding(.top, Space.s)
                }
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
        .actionBar {
            if model.circles.isEmpty {
                create
                join
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: Space.s) { join; create }
                    VStack(spacing: Space.s) { create; join }
                }
            }
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
        ("Create a circle", "Say how many people are in the room with you."),
        ("Scan one code", "Everyone scans your screen, then you all read six words aloud."),
        ("Keep talking", "Wherever you go. Messages are encrypted on each phone."),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xxl) {
            ScreenHeader(title: "Meet in person. Stay in cahoots.",
                         detail: "A private circle with the people beside you. No account, no phone number.",
                         style: .title)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(steps.enumerated()), id: \.offset) { i, step in
                    if i > 0 { Hairline() }
                    HStack(alignment: .firstTextBaseline, spacing: Space.l) {
                        Text("\(i + 1)")
                            .font(.subheadline.monospacedDigit().weight(.medium))
                            .foregroundStyle(Color.brand)
                            .frame(minWidth: 16, alignment: .leading)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(step.0).font(.headline).foregroundStyle(Color.ink)
                            Text(step.1).font(.subheadline).foregroundStyle(Color.muted)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
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
    @Environment(\.dynamicTypeSize) private var typeSize

    private var meta: String {
        var s = "\(spelled(Int(circle.members))) people. You are seat \(circle.mySeat + 1)."
        if !circle.hasRoot { s += " Needs a new ceremony to chat." }
        return s
    }

    var body: some View {
        HStack(alignment: .center, spacing: Space.m) {
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
