import Foundation

/// The phone crate's `Http`: plain requests to the relay with a 25 second timeout.
///
/// The phone crate calls this from the thread that called `Phone`, never the main thread, and
/// blocks until the answer, so the request waits on a semaphore.
final class UrlSessionHttp: Http {
    private let session: URLSession

    init(timeout: TimeInterval = 25) {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = timeout
        config.timeoutIntervalForResource = timeout
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.urlCache = nil
        config.httpCookieStorage = nil
        config.httpShouldSetCookies = false
        session = URLSession(configuration: config)
    }

    func get(url: String) throws -> Data {
        try send(try request(url, method: "GET"))
    }

    func post(url: String, body: Data) throws -> Data {
        var r = try request(url, method: "POST")
        r.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        r.httpBody = body
        return try send(r)
    }

    private func request(_ url: String, method: String) throws -> URLRequest {
        guard let u = URL(string: url) else {
            throw PhoneError.Network(message: "The relay address does not read.")
        }
        var r = URLRequest(url: u)
        r.httpMethod = method
        return r
    }

    private final class Answer: @unchecked Sendable {
        var result: Result<Data, PhoneError> = .failure(.Network(message: "Cannot reach the relay."))
    }

    private func send(_ request: URLRequest) throws -> Data {
        let done = DispatchSemaphore(value: 0)
        let answer = Answer()
        session.dataTask(with: request) { data, response, error in
            defer { done.signal() }
            if error != nil {
                answer.result = .failure(.Network(message: "Cannot reach the relay."))
                return
            }
            guard let http = response as? HTTPURLResponse else { return }
            guard (200..<300).contains(http.statusCode) else {
                answer.result = .failure(.Network(message: "The relay answered with status \(http.statusCode)."))
                return
            }
            answer.result = .success(data ?? Data())
        }.resume()
        done.wait()
        return try answer.result.get()
    }
}
