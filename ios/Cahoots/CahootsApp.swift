import SwiftUI

@main
struct CahootsApp: App {
    @StateObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase

    init() {
        let model = AppModel()
        _model = StateObject(wrappedValue: model)
        BackgroundRefresh.register(model)
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(model)
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
                ContentUnavailableView {
                    Label("Cahoots cannot open its records", systemImage: "lock")
                } description: {
                    Text(error + " Unlock the phone and try again.")
                } actions: {
                    Button("Try again") { Task { await model.open() } }
                        .buttonStyle(.borderedProminent)
                }
            } else {
                ProgressView()
            }
        }
        .task { await model.open() }
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
        }
    }
}
