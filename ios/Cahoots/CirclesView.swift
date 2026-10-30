import SwiftUI

/// The first screen: every circle this phone is a seat in.
struct CirclesView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        List {
            if model.circles.isEmpty {
                Section {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("Meet in person. Stay in cahoots.")
                            .font(.title2.weight(.semibold))
                        Text("Create a private circle with the people beside you. Then keep talking, wherever you go.")
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 8)
                }
            } else {
                Section {
                    ForEach(model.circles, id: \.id) { c in
                        NavigationLink(value: Route.chat(c.id)) { CircleRow(circle: c) }
                    }
                }
            }
            Section {
                NavigationLink(value: Route.newCircle) {
                    Label("Create a circle", systemImage: "plus.circle")
                }
                .accessibilityIdentifier("Create a circle")
                NavigationLink(value: Route.join) {
                    Label("Join a circle", systemImage: "qrcode.viewfinder")
                }
                .accessibilityIdentifier("Join a circle")
            } footer: {
                Text("Circles and history are saved on this phone. No account needed.")
            }
        }
        .navigationTitle("Cahoots")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                NavigationLink(value: Route.settings) {
                    Image(systemName: "gearshape")
                }
                .accessibilityLabel("Settings")
            }
        }
        .refreshable { _ = await model.pollAll() }
    }
}

struct CircleRow: View {
    let circle: CircleSummary

    var body: some View {
        HStack(spacing: 12) {
            CircleMark(id: circle.id, members: Int(circle.members))
            VStack(alignment: .leading, spacing: 2) {
                Text(circle.name).font(.headline)
                Text("\(circle.members) people · you are seat \(circle.mySeat + 1) · a seat on this phone")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if circle.unread > 0 {
                Text("\(circle.unread)")
                    .font(.footnote.weight(.semibold))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .background(Color.accentColor, in: Capsule())
                    .foregroundStyle(.white)
                    .accessibilityLabel("\(circle.unread) unread")
            }
        }
    }
}

/// A small mark per circle: one dot per member, coloured from the circle id.
struct CircleMark: View {
    let id: String
    let members: Int

    var body: some View {
        let hue = Double(Int(id.prefix(4), radix: 16) ?? 0) / 65535
        ZStack {
            Circle().fill(Color(hue: hue, saturation: 0.35, brightness: 0.9))
            ForEach(0..<members, id: \.self) { i in
                let a = Double(i) / Double(max(members, 1)) * 2 * .pi
                Circle()
                    .fill(Color(hue: hue, saturation: 0.7, brightness: 0.5))
                    .frame(width: 7, height: 7)
                    .offset(x: cos(a) * 11, y: sin(a) * 11)
            }
        }
        .frame(width: 40, height: 40)
        .accessibilityHidden(true)
    }
}
