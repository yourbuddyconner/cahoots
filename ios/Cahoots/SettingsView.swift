import SwiftUI

/// This phone's face and name, its key and where its secrets live, the relay for new circles,
/// and each circle's export and removal.
struct SettingsView: View {
    let core: Core
    @EnvironmentObject private var app: AppModel
    @State private var info: PhoneInfo?
    @State private var face: Data?
    @State private var name = ""
    @State private var nameNote: String?
    @State private var nameError: String?
    /// The card waits for a signature in at least one circle.
    @State private var cardWaiting = false
    @State private var working = false
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
                cardSection
                PaperSection(title: "Checking for messages") {
                    Text("iOS controls when Cahoots checks for messages in the background. The app icon’s badge shows the unread count from the last successful check.")
                        .font(.subheadline).foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.vertical, Space.m)
                }
                if !app.circles.isEmpty { circles }
                PaperSection(title: "Card check", footer: "Check whether this phone can reach the badge’s chip. This reads the chip’s version and some random bytes. It does not change any keys.") {
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
            name = (try? await core.run { $0.myName() }) ?? ""
            relay = (try? await core.run { $0.defaultRelay() }) ?? ""
            // A card left unsigned by a cancelled Face ID waits for a tap on Share again. Opening
            // Settings never asks for Face ID by itself.
            await checkCard()
            await app.refreshCard()
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

    /// The phone's own face and name, and where its keys live.
    private var phone: some View {
        let layout = typeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: Space.l))
            : AnyLayout(HStackLayout(alignment: .top, spacing: Space.l))
        return VStack(alignment: .leading, spacing: Space.l) {
            layout {
                VStack(alignment: .leading, spacing: Space.s) {
                    FaceView(pixels: face, size: 96, label: "Your face")
                    Button("Draw a new face") { Task { await redraw() } }
                        .buttonStyle(TextButtonStyle(full: false))
                        .disabled(working)
                        .accessibilityIdentifier("redraw-face")
                }
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
            nameField
            if !core.signer.usesEnclave {
                StatusLine(text: "This simulator has no Secure Enclave. The key is in the keychain.")
            }
            Text("Your circles and keys are not included in backups and cannot be transferred to a new phone. For card circles, unpair this phone on the badge, then pair the new phone. For other circles, create a new circle together.")
                .font(.footnote).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var nameField: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text("Your name")
                .font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                .accessibilityAddTraits(.isHeader)
            HStack(spacing: Space.s) {
                TextField(UIDevice.current.name, text: $name)
                    .textContentType(.name)
                    .textInputAutocapitalization(.words)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .paperField()
                    .accessibilityLabel("Your name")
                    .accessibilityIdentifier("my-name")
                    .onSubmit { Task { await saveName() } }
                Button("Save") { Task { await saveName() } }
                    .buttonStyle(CompactButtonStyle())
                    .disabled(working || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("save-name")
            }
            Text("People in your circles see this name and this face.")
                .font(.footnote).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
            StatusLine(text: nameNote, problem: false)
            StatusLine(text: nameError)
            if cardWaiting {
                if nameError == nil {
                    Text(Self.waiting)
                        .font(.footnote).foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("card-waiting")
                }
                Button("Share again") { Task { await share() } }
                    .buttonStyle(TextButtonStyle(full: false))
                    .disabled(working)
                    .accessibilityIdentifier("share-card")
            }
        }
    }

    private static let waiting = "Your name is saved on this phone. Tap Share again to share it with your circles."
    private static let notShared = "Saved on this phone, but not shared yet. Tap Share again and confirm with Face ID."

    private func saveName() async {
        guard !working else { return }
        working = true
        defer { working = false }
        nameNote = nil
        nameError = nil
        do {
            try await core.run { [name] in try $0.setMyName(name: name) }
            nameNote = "Name saved."
        } catch PhoneError.Locked {
            nameError = Self.notShared
        } catch {
            nameError = plain(error)
        }
        name = (try? await core.run { $0.myName() }) ?? name
        await checkCard()
        await app.refresh()
    }

    private func redraw() async {
        guard !working else { return }
        working = true
        defer { working = false }
        nameNote = nil
        nameError = nil
        do {
            try await core.run { try $0.redrawFace() }
        } catch PhoneError.Locked {
            nameError = Self.notShared
        } catch {
            nameError = plain(error)
        }
        face = try? await core.run { $0.myFace() }
        await checkCard()
        await app.refresh()
    }

    /// Sign and publish the card wherever it waits.
    private func share() async {
        guard !working else { return }
        working = true
        defer { working = false }
        do {
            try await core.run { try $0.shareMyCard() }
            nameError = nil
            nameNote = "Shared with your circles."
        } catch PhoneError.Locked {
            nameError = Self.notShared
        } catch {
            nameError = plain(error)
        }
        await checkCard()
    }

    private func checkCard() async {
        cardWaiting = (try? await core.run { $0.cardWaiting() }) ?? false
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
            Text("A relay delivers the circle’s encrypted messages. This address is used for new circles you create. To change it for an existing circle, open its chat, tap … at the top, then Relay address.")
                .font(.footnote).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// The badge this phone is paired with: one row to its screen, with what a tap would do,
    /// and pairing while there is none.
    private var cardSection: some View {
        PaperSection(title: "Card", footer: "Pair this phone with a badge to read and post in the badge’s circles.") {
            NavigationLink(value: Route.card) {
                HStack {
                    Label {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Card")
                            if let line = cardLine {
                                Text(line)
                                    .font(.subheadline).foregroundStyle(Color.muted)
                                    .fixedSize(horizontal: false, vertical: true)
                                    .accessibilityIdentifier("settings-card-line")
                            }
                        }
                    } icon: {
                        Image(systemName: "wave.3.right")
                    }
                    Spacer()
                    Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                }
                .foregroundStyle(Color.brand)
                .padding(.vertical, Space.s)
                .frame(minHeight: 44)
            }
            .buttonStyle(RowButtonStyle())
            .accessibilityIdentifier("settings-card")
            if app.cardStatus?.paired == false {
                Hairline()
                NavigationLink(value: Route.pairCard) {
                    HStack {
                        Label("Pair a card", systemImage: "plus")
                        Spacer()
                        Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                    }
                    .foregroundStyle(Color.brand)
                    .frame(minHeight: 44)
                }
                .buttonStyle(RowButtonStyle())
                .accessibilityIdentifier("settings-pair-card")
            }
        }
    }

    /// The first line of what a tap would do, or "not paired". Nil until the card is read.
    private var cardLine: String? {
        guard let status = app.cardStatus else { return nil }
        guard status.paired else { return "not paired" }
        return CardView.summary(status) { app.seatName(circle: $0, seat: $1) }.first
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
