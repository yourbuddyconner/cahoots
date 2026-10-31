import SwiftUI

/// The circle's roster and the private names kept on this phone.
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

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                VStack(alignment: .leading, spacing: Space.m) {
                    FaceRow(faces: members.compactMap { faces[$0.seat] }, size: 32)
                    ScreenHeader(title: app.circle(circle)?.name ?? "Your circle",
                                 detail: "Give each seat a familiar name. Open someone’s hello words when you want to check who you’re talking to.", style: .title)
                }
                if !loaded && error == nil {
                    ProgressView("Loading people…").frame(maxWidth: .infinity)
                }
                if !members.isEmpty {
                    PaperSection(title: "\(members.count) people", footer: "Names are saved only on this phone. Other members choose their own names for each seat.") {
                        ForEach(Array(members.enumerated()), id: \.element.seat) { i, member in
                            if i > 0 { Hairline() }
                            HStack(spacing: Space.s) {
                                if member.mine {
                                    Button { editingSeat = member.seat } label: { row(member) }
                                        .buttonStyle(RowButtonStyle())
                                } else {
                                    NavigationLink(value: Route.hello(circle, member.seat)) { row(member) }
                                        .buttonStyle(RowButtonStyle())
                                        .accessibilityIdentifier("hello-seat-\(member.seat)")
                                }
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
                    DisclosureGroup("About these devices") {
                        VStack(alignment: .leading, spacing: Space.l) {
                            ForEach(members, id: \.seat) { member in
                                VStack(alignment: .leading, spacing: Space.xs) {
                                    Text(app.seatName(circle: circle, seat: member.seat)).font(.subheadline.weight(.semibold))
                                    Text(Self.tier(member.tier)).font(.subheadline).foregroundStyle(Color.muted)
                                }
                            }
                            Text("These are the protection levels each device reported when the circle was created. They have not been independently verified. Every member holds the circle’s secret.")
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
                faces = try await core.run { [circle, members] in
                    $0.faces(circle: circle, members: UInt8(members.count))
                }
                loaded = true
            } catch { self.error = plain(error) }
        }
    }

    private func row(_ member: Member) -> some View {
        HStack(spacing: Space.m) {
            FaceView(pixels: faces[member.seat], size: 44, label: "Seat \(member.seat + 1)'s face")
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(app.seatName(circle: circle, seat: member.seat))
                    .font(.headline).foregroundStyle(Color.ink)
                    .fixedSize(horizontal: false, vertical: true)
                Text(member.mine ? "Seat \(member.seat + 1). You" : "Seat \(member.seat + 1). Hello words")
                    .font(.subheadline).foregroundStyle(Color.muted)
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

/// A call check with one other seat. Both phones advance their own counter.
struct HelloView: View {
    let core: Core
    let circle: String
    let seat: UInt8
    @EnvironmentObject private var app: AppModel
    @State private var words: HelloWords?
    @State private var face: Data?
    @State private var error: String?
    @State private var notice: String?
    @State private var advancing = false
    @State private var confirmNext = false

    private var who: String { app.seatName(circle: circle, seat: seat) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                HStack(spacing: Space.m) {
                    FaceView(pixels: face, size: 44, label: "\(who)'s face")
                    ScreenHeader(title: "Check with \(who)",
                                 detail: "Ask them to open your seat (seat \((app.circle(circle)?.mySeat ?? 0) + 1)) in this circle’s People.", style: .title2)
                }
                if let words {
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
                    VStack(alignment: .leading, spacing: 0) {
                        Hairline()
                        said("1", "You say aloud", words.mine, detail: "They check that these match on their screen.")
                        Hairline()
                        said("2", "Listen for their reply", words.theirs, detail: "Let them say these. Don’t give away the answer.")
                        Hairline()
                    }
                    InstructionStep(number: 3, title: "Finished? Move on together",
                                    detail: "If both pairs match, each of you taps Next hello to prepare fresh words for your next conversation. The other phone won’t advance automatically.")
                    DisclosureGroup("Numbers or words don’t match?") {
                        Text("Check that you’re in the same circle and have opened each other’s seat. If hello numbers differ, only the person with the lower number taps Next hello until they match. If the words still differ, stop the check. A familiar voice alone is not enough.")
                            .font(.subheadline).foregroundStyle(Color.muted)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.top, Space.s)
                    }
                    .foregroundStyle(Color.ink)
                } else if error == nil {
                    ProgressView("Loading hello words…").frame(maxWidth: .infinity)
                }
                StatusLine(text: notice, problem: false)
                StatusLine(text: error)
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .paperBackground()
        .navigationTitle("Hello words")
        .navigationBarTitleDisplayMode(.large)
        .actionBar {
            Button { confirmNext = true } label: { Working(busy: advancing, title: "Next hello") }
                .buttonStyle(.filled)
                .accessibilityLabel("Next hello")
                .disabled(words == nil || advancing)
        }
        .confirmationDialog("Move to the next hello?", isPresented: $confirmNext, titleVisibility: .visible) {
            Button("Move to next hello") { Task { await next() } }
        } message: {
            Text("This changes only your phone. Ask \(who) to advance too, unless you’re catching up to their number. You can’t go back to earlier words.")
        }
        .task {
            do {
                face = try? await core.run { [circle, seat] in $0.seatFace(circle: circle, seat: seat) }
                words = try await core.run { [circle, seat] in try $0.helloWords(circle: circle, otherSeat: seat) }
            } catch { self.error = plain(error) }
        }
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

    private func next() async {
        guard !advancing else { return }
        advancing = true
        error = nil
        defer { advancing = false }
        do {
            let updated = try await core.run { [circle, seat] in try $0.nextHello(circle: circle, otherSeat: seat) }
            words = updated
            notice = "Ready for hello \(UInt64(updated.number) + 1). Check that both screens show this number."
        } catch { self.error = plain(error) }
    }
}
