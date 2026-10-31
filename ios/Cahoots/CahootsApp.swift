import SwiftUI
import UIKit

@main
struct CahootsApp: App {
    @StateObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase

    init() {
        let model = AppModel()
        _model = StateObject(wrappedValue: model)
        BackgroundRefresh.register(model)
        Self.styleBars()
    }

    /// Large titles in the system serif, as the site's Georgia headlines. Only the title fonts
    /// and colours change, so the bars keep their system material (glass on iOS 26).
    private static func styleBars() {
        let ink = UIColor(named: "Ink") ?? .label
        let large = UIFont.preferredFont(forTextStyle: .largeTitle)
        let serif = large.fontDescriptor.withDesign(.serif) ?? large.fontDescriptor
        let bar = UINavigationBar.appearance()
        bar.largeTitleTextAttributes = [.font: UIFont(descriptor: serif, size: 0), .foregroundColor: ink]
        bar.titleTextAttributes = [.foregroundColor: ink]
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(model)
                .tint(Color.brand)
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background { BackgroundRefresh.schedule() }
        }
    }
}

struct RootView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            if let core = model.core {
                NavigationStack(path: $model.path) {
                    CirclesView()
                        .navigationDestination(for: Route.self) { route in
                            destination(route, core: core)
                        }
                }
            } else if let error = model.openError {
                VStack(alignment: .leading, spacing: Space.xl) {
                    Spacer()
                    Image(systemName: "lock").font(.largeTitle).foregroundStyle(Color.ink)
                        .accessibilityHidden(true)
                    ScreenHeader(title: "Cahoots cannot open its records",
                                 detail: error + " Unlock the phone and try again.", style: .title)
                    Spacer()
                    Button("Try again") { Task { await model.open() } }
                        .buttonStyle(.filled)
                }
                .padding(Space.l)
                .paperBackground()
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity).paperBackground()
            }
        }
        .task { await model.open() }
        // A channel link: tapped in another app, or a universal link to cahoots.network/app.
        .onOpenURL { model.openLink($0) }
        .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
            if let url = activity.webpageURL { model.openLink(url) }
        }
        .alert("Ceremony lost", isPresented: Binding(get: { model.lostCeremony != nil },
                                                     set: { if !$0 { model.lostCeremony = nil } })) {
            Button("OK") { model.lostCeremony = nil }
        } message: {
            Text(model.lostCeremony ?? "")
        }
        // While the app is open, read every circle's relay now and then.
        .task(id: scenePhase) {
            guard scenePhase == .active else { return }
            while !Task.isCancelled {
                _ = await model.pollAll()
                try? await Task.sleep(for: .seconds(30))
            }
        }
    }

    @ViewBuilder
    private func destination(_ route: Route, core: Core) -> some View {
        switch route {
        case .newCircle: NewCircleView()
        case .join: JoinView(core: core)
        case let .ceremony(id): CeremonyView(core: core, id: id)
        case let .chat(id): ChatView(core: core, circle: id)
        case let .people(id): PeopleView(core: core, circle: id)
        case let .hello(id, seat): HelloView(core: core, circle: id, seat: seat)
        case .settings: SettingsView(core: core)
        case let .channel(id): ChannelView(core: core, id: id)
        case let .joinChannel(link): JoinChannelView(core: core, link: link)
        case let .channelMembers(id): ChannelMembersView(core: core, id: id)
        case let .channelRequests(id): ChannelRequestsView(core: core, id: id)
        }
    }
}
