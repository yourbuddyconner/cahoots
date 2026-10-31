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
    @State private var showingGuide = false
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxl) {
                Button { showingGuide = true } label: {
                    HStack {
                        Label("How Cahoots works", systemImage: "book")
                        Spacer()
                        Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                    }
                    .foregroundStyle(Color.brand)
                    .frame(minHeight: 44)
                }
                .buttonStyle(RowButtonStyle())
                phone
                relaySection
                PaperSection(title: "Checking for messages") {
                    Text("iOS controls when Cahoots checks for messages in the background. The app icon’s badge shows the unread count from the last successful check.")
                        .font(.subheadline).foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.vertical, Space.m)
                }
                if !app.circles.isEmpty { circles }
                PaperSection(title: "Bench", footer: "For trying a badge’s chip over NFC. It reads the chip’s version and random bytes and touches no key.") {
                    NavigationLink {
                        NfcDiagnosticsView()
                    } label: {
                        HStack {
                            Label("NFC check", systemImage: "wave.3.right")
                            Spacer()
                            Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                        }
                        .foregroundStyle(Color.brand)
                        .frame(minHeight: 44)
                    }
                    .buttonStyle(RowButtonStyle())
                    .accessibilityIdentifier("nfc-check")
                }
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
        .sheet(isPresented: $showingGuide) {
            WelcomeView(core: core, firstVisit: false) { showingGuide = false }
        }
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
        .confirmationDialog("Remove \(removing?.name ?? "this circle") from this phone? Its messages and keys will be deleted from this phone. You cannot undo this. Other members keep their copies.",
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
            Text("Your circles and keys stay on this phone and are not included in backups. You can’t transfer them to a new phone. To use another phone, create a new circle together.")
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
            Text("The host chooses the relay used during setup and for the circle’s chat. This setting applies to new circles you create. To change an existing circle’s relay, open its conversation settings.")
                .font(.footnote).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var circles: some View {
        PaperSection(title: "Circles", footer: "Exports contain readable messages. Anyone with the exported file can read them. Encryption keys are not included.") {
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
