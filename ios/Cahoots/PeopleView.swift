import SwiftUI

/// The circle's roster: the names and faces on the members' cards, and for a seat without a
/// card, the name kept on this phone.
struct PeopleView: View {
    let core: Core
    let circle: String
    @EnvironmentObject private var app: AppModel
    @State private var members: [Member] = []
    @State private var faces: [UInt8: Data] = [:]
    @State private var error: String?
    @State private var editingSeat: UInt8?
    @State private var loaded = false

    static func tier(_ t: UInt8) -> String {
        switch t {
        case 0: return "Tier 0, software only"
        case 1: return "Tier 1, an open microcontroller"
        case 2: return "Tier 2, a sealed microcontroller"
        case 3: return "Tier 3, a hardware key store"
        case 4: return "Tier 4, inspected hardware"
        default: return "Tier \(t), unknown to this app"
        }
    }

    /// The kind of device on a seat's card, in words, or nil without a card that says.
    static func device(_ d: UInt8?) -> String? {
        switch d {
        case 0: return "on a phone"
        case 1: return "on a badge"
        case 2: return "in a browser"
        case 3: return "on a laptop"
        default: return nil
        }
    }

    /// The tier, then the kind of device when the card says. In a card circle this phone's
    /// seat is its card's, and says so.
    static func about(_ member: Member, card: Bool = false) -> String {
        let device = card && member.mine ? "your card" : device(member.device)
        return [tier(member.tier), device].compactMap { $0 }.joined(separator: ", ")
    }

    /// A circle from a badge this phone is paired with.
    private var card: Bool { app.circle(circle).map(AppModel.isCard) ?? false }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                VStack(alignment: .leading, spacing: Space.m) {
                    FaceRow(faces: members.compactMap { faces[$0.seat] }, size: 32)
                    ScreenHeader(title: app.circle(circle)?.name ?? "Your circle",
                                 detail: "Each seat is a device in this circle. On a call, open the other person’s words. Take turns saying your two words and checking the reply.", style: .title)
                }
                if !loaded && error == nil {
                    ProgressView("Loading people…").frame(maxWidth: .infinity)
                }
                if !members.isEmpty {
                    PaperSection(title: "\(members.count) people", footer: "Each person chooses their own name and face. For a seat without a name, tap the pencil to add one that only you see.") {
                        ForEach(Array(members.enumerated()), id: \.element.seat) { i, member in
                            if i > 0 { Hairline() }
                            let named = app.hasCardName(circle: circle, seat: member.seat)
                            HStack(spacing: Space.s) {
                                if member.mine && card {
                                    // The seat is the badge's, named on the badge.
                                    row(member)
                                } else if member.mine && named {
                                    NavigationLink(value: Route.settings) { row(member) }
                                        .buttonStyle(RowButtonStyle())
                                } else if member.mine {
                                    Button { editingSeat = member.seat } label: { row(member) }
                                        .buttonStyle(RowButtonStyle())
                                } else {
                                    NavigationLink(value: Route.hello(circle, member.seat)) { row(member) }
                                        .buttonStyle(RowButtonStyle())
                                        .accessibilityIdentifier("hello-seat-\(member.seat)")
                                }
                                if !named && !(member.mine && card) {
                                    Button { editingSeat = member.seat } label: {
                                        Image(systemName: "pencil")
                                            .font(.body)
                                            .frame(width: 44, height: 44)
                                            .contentShape(Rectangle())
                                    }
                                    .buttonStyle(.plain)
                                    .foregroundStyle(Color.brand)
                                    .accessibilityLabel("Edit name for seat \(member.seat + 1)")
                                    .accessibilityIdentifier("edit-seat-\(member.seat)")
                                }
                            }
                        }
                    }
                    DisclosureGroup("About these devices") {
                        VStack(alignment: .leading, spacing: Space.l) {
                            ForEach(members, id: \.seat) { member in
                                VStack(alignment: .leading, spacing: Space.xs) {
                                    Text(app.seatName(circle: circle, seat: member.seat)).font(.subheadline.weight(.semibold))
                                    Text(Self.about(member, card: card)).font(.subheadline).foregroundStyle(Color.muted)
                                }
                            }
                            Text("These are the protection levels each device reported when the circle was made. Nothing checked them. Every member holds the circle’s secret.")
                                .font(.footnote).foregroundStyle(Color.muted)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, Space.m)
                    }
                    .foregroundStyle(Color.ink)
                }
                StatusLine(text: error)
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .paperBackground()
        .navigationTitle("People")
        .navigationBarTitleDisplayMode(.large)
        .sheet(isPresented: Binding(get: { editingSeat != nil }, set: { if !$0 { editingSeat = nil } })) {
            if let seat = editingSeat {
                SeatNameSheet(circle: circle, seat: seat, face: faces[seat])
            }
        }
        .task {
            do {
                members = try await core.run { [circle] in try $0.circleMembers(circle: circle) }
                app.noteMembers(members, circle: circle)
                faces = try await core.run { [circle, members] in
                    $0.faces(circle: circle, members: UInt8(members.count))
                }
                loaded = true
            } catch { self.error = plain(error) }
        }
    }

    private func row(_ member: Member) -> some View {
        let name = app.seatName(circle: circle, seat: member.seat)
        return HStack(spacing: Space.m) {
            FaceView(pixels: faces[member.seat], size: 44, label: "\(name)'s face")
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(name)
                    .font(.headline).foregroundStyle(Color.ink)
                    .fixedSize(horizontal: false, vertical: true)
                Text(member.mine ? "Seat \(member.seat + 1). You" : "Seat \(member.seat + 1). Hello words")
                    .font(.subheadline).foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
                Text(Self.about(member, card: card))
                    .font(.footnote).foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            if !member.mine {
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold)).foregroundStyle(Color.muted)
                    .accessibilityHidden(true)
            }
        }
        .padding(.vertical, Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

struct SeatNameSheet: View {
    let circle: String
    let seat: UInt8
    let face: Data?
    @EnvironmentObject private var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var error: String?
    @FocusState private var focused: Bool

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xl) {
                    HStack(spacing: Space.l) {
                        FaceView(pixels: face, size: 64)
                        ScreenHeader(title: "Who is seat \(seat + 1)?",
                                     detail: "Choose a name you’ll recognize in this circle.", style: .title2)
                    }
                    VStack(alignment: .leading, spacing: Space.s) {
                        Text("Name on this phone").font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                        TextField("Seat \(seat + 1)", text: $name)
                            .textContentType(.nickname)
                            .textInputAutocapitalization(.words)
                            .autocorrectionDisabled()
                            .submitLabel(.done)
                            .focused($focused)
                            .paperField()
                            .accessibilityIdentifier("seat-name")
                            .onSubmit { save() }
                        Text("Only you see this name. Leave it blank to use the seat number.")
                            .font(.footnote).foregroundStyle(Color.muted)
                    }
                    StatusLine(text: error)
                }
                .padding(Space.l)
            }
            .paperBackground()
            .navigationTitle("Edit name")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { save() }.fontWeight(.semibold).accessibilityIdentifier("save-seat-name")
                }
            }
            .onAppear {
                name = app.seatNames[circle]?[String(seat)] ?? ""
                focused = true
            }
        }
    }

    private func save() {
        do {
            try app.setSeatName(name, circle: circle, seat: seat)
            dismiss()
        } catch { self.error = "Couldn’t save this name. Try again." }
    }
}

/// The hello words of one seat, as `HelloModel` asks for them. `Core` answers on the phone's
/// queue; the tests stand in for it.
protocol HelloSource: Sendable {
    func helloWords(circle: String, seat: UInt8) async throws -> HelloWords
    func nextHello(circle: String, seat: UInt8) async throws -> HelloWords
    /// One tap on the card: Face ID first, then the reader's sheet.
    func tapCard() async throws -> TapReport
}

extension Core: HelloSource {
    func helloWords(circle: String, seat: UInt8) async throws -> HelloWords {
        try await run { try $0.helloWords(circle: circle, otherSeat: seat) }
    }

    func nextHello(circle: String, seat: UInt8) async throws -> HelloWords {
        try await run { try $0.nextHello(circle: circle, otherSeat: seat) }
    }

    func tapCard() async throws -> TapReport {
        try await run { try $0.tapCard() }
    }
}

/// The words of the current hello with one seat, the move to the next, and in a card circle
/// the tap that brings more words when the phone has none left.
@MainActor
final class HelloModel: ObservableObject {
    let source: HelloSource
    let circle: String
    let seat: UInt8
    @Published private(set) var words: HelloWords?
    @Published private(set) var error: String?
    @Published private(set) var notice: String?
    @Published private(set) var advancing = false
    @Published private(set) var tapping = false
    /// A card circle has no hello words left with this seat: a tap on the card brings more.
    @Published private(set) var needsTap = false
    /// Next hello asked for the tap. The person confirmed it already, so it goes on once the
    /// tap is through.
    private var nextAfterTap = false

    init(source: HelloSource, circle: String, seat: UInt8) {
        self.source = source
        self.circle = circle
        self.seat = seat
    }

    /// The words for the current hello number. A card circle with none left asks for a tap.
    func load() async {
        do {
            words = try await source.helloWords(circle: circle, seat: seat)
            needsTap = false
            error = nil
        } catch PhoneError.NeedsTap {
            needsTap = true
            self.error = plain(PhoneError.NeedsTap)
        } catch {
            self.error = plain(error)
        }
    }

    /// Move to the next hello. Past the last words from the card the number stays, and the
    /// screen asks for a tap.
    func next() async {
        guard !advancing else { return }
        advancing = true
        defer { advancing = false }
        error = nil
        do {
            let updated = try await source.nextHello(circle: circle, seat: seat)
            words = updated
            notice = "Ready for hello \(UInt64(updated.number) + 1). Check that both screens show this number."
        } catch PhoneError.NeedsTap {
            needsTap = true
            nextAfterTap = true
            notice = nil
            self.error = plain(PhoneError.NeedsTap)
        } catch {
            self.error = plain(error)
        }
    }

    /// Tap the card for more hello words and read the words again, then move to the next
    /// hello if that is what asked for the tap.
    func tapCard() async {
        guard !tapping else { return }
        tapping = true
        defer { tapping = false }
        error = nil
        notice = nil
        do {
            _ = try await source.tapCard()
        } catch {
            self.error = plain(error)
            return
        }
        await load()
        if nextAfterTap, !needsTap, error == nil {
            nextAfterTap = false
            await next()
        }
    }
}

/// A call check with one other seat. Both phones advance their own counter.
struct HelloView: View {
    let core: Core
    let circle: String
    let seat: UInt8
    @EnvironmentObject private var app: AppModel
    @StateObject private var model: HelloModel
    @State private var face: Data?
    @State private var confirmNext = false

    init(core: Core, circle: String, seat: UInt8) {
        self.core = core
        self.circle = circle
        self.seat = seat
        _model = StateObject(wrappedValue: HelloModel(source: core, circle: circle, seat: seat))
    }

    private var who: String { app.seatName(circle: circle, seat: seat) }

    /// In a card circle, the line that says the hello words from the card run low: 3 or fewer.
    static func pocketNote(_ left: UInt32?) -> String? {
        guard let left, left <= 3 else { return nil }
        return "\(left) \(left == 1 ? "hello" : "hellos") left from your card. Tap the card soon."
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                HStack(spacing: Space.m) {
                    FaceView(pixels: face, size: 44, label: "\(who)'s face")
                    ScreenHeader(title: "Check with \(who)",
                                 detail: "Ask them to open this circle on their phone, tap the chat title to open People, then choose your seat (seat \((app.circle(circle)?.mySeat ?? 0) + 1)).", style: .title2)
                }
                if let words = model.words {
                    VStack(alignment: .leading, spacing: Space.s) {
                        Text("Hello \(UInt64(words.number) + 1)")
                            .font(.title3.weight(.semibold).monospacedDigit()).foregroundStyle(Color.brand)
                            .accessibilityIdentifier("hello-number")
                        Text("Check that both screens show this number.")
                            .font(.subheadline).foregroundStyle(Color.muted)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(Space.l)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.brand.opacity(0.06), in: RoundedRectangle(cornerRadius: Radius.control))
                    StatusLine(text: model.notice, problem: false)
                    if !model.needsTap, let low = Self.pocketNote(words.cardLeft) {
                        VStack(alignment: .leading, spacing: 0) {
                            StatusLine(text: low, problem: false)
                            Button("Tap the card") { Task { await tap() } }
                                .buttonStyle(TextButtonStyle(full: false))
                                .disabled(model.tapping || model.advancing)
                                .accessibilityIdentifier("hello-tap-card-soon")
                        }
                        .accessibilityIdentifier("hello-pocket-low")
                    }
                    VStack(alignment: .leading, spacing: 0) {
                        Hairline()
                        said("1", "You say aloud", words.mine, detail: "Say these two words. Ask them to check the reply on their screen.")
                        Hairline()
                        said("2", "Listen for their reply", words.theirs, detail: "Ask them to say their two words. Check that you hear these words. Do not tell them what you expect to hear.")
                        Hairline()
                    }
                    InstructionStep(number: 3, title: "When both pairs match",
                                    detail: "After both pairs match, each of you taps Next hello to get the words for your next call. Tapping changes only your phone.")
                    DisclosureGroup("Numbers or words don’t match?") {
                        Text("Check that you’re in the same circle and have opened each other’s seat. If the numbers differ, only the person with the lower number taps Next hello until they match. If the words still differ, stop. You have not confirmed who is on the call, even if the voice sounds familiar.")
                            .font(.subheadline).foregroundStyle(Color.muted)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.top, Space.s)
                    }
                    .foregroundStyle(Color.ink)
                } else if model.error == nil {
                    ProgressView("Loading hello words…").frame(maxWidth: .infinity)
                }
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .paperBackground()
        .navigationTitle("Hello words")
        .navigationBarTitleDisplayMode(.large)
        .actionBar {
            // A problem shows above the button, where it is seen, as in the chat.
            StatusLine(text: model.error)
            if model.needsTap {
                Button { Task { await tap() } } label: { Working(busy: model.tapping, title: "Tap the card") }
                    .buttonStyle(.filled)
                    .accessibilityLabel("Tap the card")
                    .accessibilityIdentifier("hello-tap-card")
                    .disabled(model.tapping)
            } else {
                Button { confirmNext = true } label: { Working(busy: model.advancing, title: "Next hello") }
                    .buttonStyle(.filled)
                    .accessibilityLabel("Next hello")
                    .disabled(model.words == nil || model.advancing || model.tapping)
            }
        }
        .confirmationDialog("Move to the next hello?", isPresented: $confirmNext, titleVisibility: .visible) {
            Button("Move to next hello") { Task { await model.next() } }
        } message: {
            Text("This changes only your phone. Ask \(who) to tap Next hello too, unless you’re catching up to their number. You can’t go back to earlier words.")
        }
        .task {
            face = try? await core.run { [circle, seat] in $0.seatFace(circle: circle, seat: seat) }
            await model.load()
        }
    }

    /// Tap the card, then read the circles and the card's status again for the other screens.
    private func tap() async {
        await model.tapCard()
        await app.refresh()
    }

    private func said(_ number: String, _ label: String, _ word: String, detail: String) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text("\(number). \(label)").font(.subheadline.weight(.semibold)).foregroundStyle(Color.muted)
            // Two words can wrap at large text sizes instead of shrinking.
            Text(word).serifTitle(.largeTitle).fixedSize(horizontal: false, vertical: true)
                .textSelection(.disabled)
            Text(detail).font(.footnote).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, Space.l)
        .accessibilityElement(children: .combine)
    }
}
