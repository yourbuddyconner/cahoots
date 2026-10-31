import Foundation

/// The phone crate's `Http`: plain requests to the relay with a 25 second timeout.
/// `get` and `post` throw on a refusal; `request` returns the status and the body.
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
        try send(try urlRequest(url, method: "GET"))
    }

    func post(url: String, body: Data) throws -> Data {
        var r = try urlRequest(url, method: "POST")
        r.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        r.httpBody = body
        return try send(r)
    }

    /// Any method, any headers, for the channel calls. Every answer comes back with its status,
    /// a refusal included, so the phone crate reads 401, 404 and 410 itself. Only a request that
    /// got no answer at all throws.
    func request(method: String, url: String, headers: [HttpHeader], body: Data?) throws -> HttpResponse {
        var r = try urlRequest(url, method: method)
        for h in headers { r.setValue(h.value, forHTTPHeaderField: h.name) }
        if let body {
            if r.value(forHTTPHeaderField: "Content-Type") == nil {
                r.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
            }
            r.httpBody = body
        }
        return try exchange(r)
    }

    private func urlRequest(_ url: String, method: String) throws -> URLRequest {
        guard let u = URL(string: url) else {
            throw PhoneError.Network(message: "The relay address is invalid. Check the address and try again.")
        }
        var r = URLRequest(url: u)
        r.httpMethod = method
        return r
    }

    /// The words for a request that never got an answer: a timeout, no connection. Screens that
    /// retry on their own add "Trying again." to these words and to no others.
    static let unreachable = "Cannot reach the relay."

    /// The words for a relay that answered with an error: its `detail` when the body has one.
    static func refusal(status: Int, body: Data?, method: String) -> String {
        if let body,
           let json = try? JSONSerialization.jsonObject(with: body) as? [String: Any],
           let detail = json["detail"] as? String, !detail.isEmpty {
            return "The relay refused the \(method == "POST" ? "message" : "request"): \(detail)"
        }
        return "The relay returned an error (HTTP \(status))."
    }

    private final class Exchange: @unchecked Sendable {
        var result: Result<HttpResponse, PhoneError> = .failure(.Network(message: UrlSessionHttp.unreachable))
    }

    private func exchange(_ request: URLRequest) throws -> HttpResponse {
        let done = DispatchSemaphore(value: 0)
        let answer = Exchange()
        session.dataTask(with: request) { data, response, error in
            defer { done.signal() }
            guard error == nil, let http = response as? HTTPURLResponse else { return }
            answer.result = .success(HttpResponse(status: UInt16(clamping: http.statusCode), body: data ?? Data()))
        }.resume()
        done.wait()
        return try answer.result.get()
    }

    private final class Answer: @unchecked Sendable {
        var result: Result<Data, PhoneError> = .failure(.Network(message: UrlSessionHttp.unreachable))
    }

    private func send(_ request: URLRequest) throws -> Data {
        let done = DispatchSemaphore(value: 0)
        let answer = Answer()
        session.dataTask(with: request) { data, response, error in
            defer { done.signal() }
            if error != nil {
                answer.result = .failure(.Network(message: Self.unreachable))
                return
            }
            guard let http = response as? HTTPURLResponse else { return }
            guard (200..<300).contains(http.statusCode) else {
                let words = Self.refusal(status: http.statusCode, body: data, method: request.httpMethod ?? "GET")
                answer.result = .failure(.Network(message: words))
                return
            }
            answer.result = .success(data ?? Data())
        }.resume()
        done.wait()
        return try answer.result.get()
    }
}
