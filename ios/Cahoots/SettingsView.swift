import SwiftUI

/// This phone's key, where its secrets live, the relay for new circles, and each circle's
/// export and removal.
struct SettingsView: View {
    let core: Core
    @EnvironmentObject private var app: AppModel
    @State private var info: PhoneInfo?
    @State private var face: Data?
    @State private var relay = ""
    @State private var relayNote: String?
    @State private var relayError: String?
    @State private var export: ExportText?
    @State private var removing: CircleSummary?
    @State private var error: String?
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxl) {
                phone
                relaySection
                PaperSection(title: "Checking for messages") {
                    Text("While Cahoots is closed, iOS decides when it may check for new messages. The number on the icon is how many are unread.")
                        .font(.subheadline).foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.vertical, Space.m)
                }
                if !app.circles.isEmpty { circles }
                if let info {
                    Text("Cahoots \(info.appVersion)")
                        .font(.footnote).foregroundStyle(Color.muted)
                        .frame(maxWidth: .infinity, alignment: .center)
                }
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .scrollDismissesKeyboard(.interactively)
        .paperBackground()
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.large)
        .task {
            info = try? await core.run { $0.describe() }
            face = try? await core.run { $0.myFace() }
            relay = (try? await core.run { $0.defaultRelay() }) ?? ""
        }
        .sheet(item: $export) { e in
            NavigationStack {
                ScrollView {
                    Text(e.text).font(.footnote.monospaced()).foregroundStyle(Color.ink)
                        .padding(Space.l).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .paperBackground()
                .navigationTitle("History")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Close") { export = nil } }
                    ToolbarItem(placement: .primaryAction) { ShareLink(item: e.text) }
                }
            }
        }
        .confirmationDialog("Remove \(removing?.name ?? "this circle") from this phone? Other members keep theirs. This phone's seat key for it is deleted.",
                            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                            titleVisibility: .visible) {
            Button("Remove circle", role: .destructive) {
                guard let c = removing else { return }
                Task {
                    do { try await app.delete(c.id) } catch { self.error = plain(error) }
                }
            }
        }
    }

    /// The phone's own face, and where its keys live.
    private var phone: some View {
        let layout = typeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: Space.l))
            : AnyLayout(HStackLayout(alignment: .top, spacing: Space.l))
        return VStack(alignment: .leading, spacing: Space.l) {
            layout {
                FaceView(pixels: face, size: 96, label: "This phone's face")
                VStack(alignment: .leading, spacing: Space.s) {
                    Text("This phone").serifTitle(.title2)
                    if let info {
                        Text(info.keyStore.prefix(1).uppercased() + info.keyStore.dropFirst() + ".")
                            .font(.subheadline).foregroundStyle(Color.ink)
                            .fixedSize(horizontal: false, vertical: true)
                        Text(info.fingerprint)
                            .font(.footnote.monospaced())
                            .foregroundStyle(Color.muted)
                            .textSelection(.enabled)
                            .accessibilityLabel("Key fingerprint \(info.fingerprint)")
                    }
                }
            }
            if !core.signer.usesEnclave {
                StatusLine(text: "This simulator has no Secure Enclave. The key is in the keychain.")
            }
            Text("This phone's seats do not move. Seat keys never leave this phone, and backups leave out every circle. A new phone means new circles.")
                .font(.footnote).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var relaySection: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text("Relay for new circles")
                .font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                .accessibilityAddTraits(.isHeader)
            HStack(spacing: Space.s) {
                TextField("https://relay.example.com", text: $relay)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.subheadline.monospaced())
                    .paperField()
                    .accessibilityLabel("Relay for new circles")
                    .accessibilityIdentifier("default-relay")
                Button("Save") { Task { await saveRelay() } }
                    .buttonStyle(CompactButtonStyle())
                    .accessibilityIdentifier("save-relay")
            }
            StatusLine(text: relayNote, problem: false)
            StatusLine(text: relayError)
            Text("A ceremony held through the relay uses the host's relay, and the circle keeps it. Change a circle's relay from its chat.")
                .font(.footnote).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var circles: some View {
        PaperSection(title: "Circles", footer: "Export is a readable copy of the history. Nothing secret is in it.") {
            ForEach(Array(app.circles.enumerated()), id: \.element.id) { i, c in
                if i > 0 { Hairline() }
                VStack(alignment: .leading, spacing: Space.xs) {
                    Text(c.name).serifTitle(.title3)
                    Text(c.relay).font(.footnote.monospaced()).foregroundStyle(Color.muted)
                        .lineLimit(1).truncationMode(.middle)
                    HStack(spacing: Space.xl) {
                        Button("Export history") { Task { await exportHistory(c) } }
                            .buttonStyle(TextButtonStyle(full: false))
                        Button("Remove") { removing = c }
                            .buttonStyle(TextButtonStyle(full: false, color: .ink))
                            .accessibilityLabel("Remove \(c.name)")
                    }
                }
                .padding(.vertical, Space.m)
            }
            StatusLine(text: error)
        }
    }

    private func saveRelay() async {
        do {
            try await core.run { [relay] in try $0.setDefaultRelay(url: relay) }
            relay = (try? await core.run { $0.defaultRelay() }) ?? relay
            relayError = nil
            relayNote = "Saved. New circles use this relay."
        } catch {
            relayNote = nil
            relayError = plain(error)
        }
    }

    private func exportHistory(_ c: CircleSummary) async {
        do {
            let text = try await core.run { try $0.exportHistory(circle: c.id) }
            export = ExportText(text: text)
        } catch {
            self.error = plain(error)
        }
    }
}

struct ExportText: Identifiable {
    let id = UUID()
    let text: String
}
