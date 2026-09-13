---
name: web-security
description: Use defensive security practice whenever building or reviewing anything that faces untrusted input - authentication flows, session cookies, JWTs, OAuth, authorization checks, CORS, security headers, and input handling. Covers OWASP Top 10 prevention with concrete code - parameterized queries, output encoding, CSP, PKCE, least privilege, rate limiting. Triggers on login and permission logic, secrets near logs, XSS or injection risk, and API exposure decisions.
---

# Web Security

Every input is hostile until validated at a trust boundary, and every output is dangerous until encoded for its context. Systems fail open by default; these rules exist to flip the default to deny.

## OWASP Top 10 - Concrete Prevention

1. **Injection** - never build queries from strings:

```python
cursor.execute("SELECT * FROM users WHERE id = %s", (user_id,))  # parameterized
```

   ORMs (SQLAlchemy, Django, Prisma) parameterize by default; raw string interpolation is the bug.

2. **Broken authentication** - support MFA, rotate the session ID at login, hash passwords with bcrypt/argon2 (never bare MD5/SHA - they are too fast, which helps attackers brute-force).
3. **Sensitive data exposure** - TLS everywhere including internal hops, encrypt at rest, never log secrets or full tokens.
4. **XXE** - disable XML entity processing when parsing untrusted XML:

```python
from defusedxml import ElementTree   # entities disabled by default
```

5. **Broken access control** - deny by default; verify *on every request*, server-side. An `isAdmin` flag in the browser is decoration.
6. **Security misconfiguration** - debug off in prod, defaults changed, headers set (below).
7. **XSS** - escape output by context, add CSP as a second line of defense, sanitize input with allowlists.
8. **Insecure deserialization** - never `unpickle` or deserialize untrusted data into objects; use JSON and validate the schema.
9. **Known vulnerabilities** - scan dependencies in CI (`cargo audit`, `npm audit`, trivy); monitor CVE feeds for your stack.
10. **Insufficient logging** - log logins, permission failures, and validation rejects; alert on anomalies (a burst of 401s is credential stuffing).

## Authentication

**Session-based** - the cookie is the credential; make it unstealable:

```
Set-Cookie: session=abc123; HttpOnly; Secure; SameSite=Lax
```

- `HttpOnly` hides it from JavaScript (XSS cannot steal it), `Secure` confines it to HTTPS, `SameSite=Lax` blocks cross-site CSRF sends.
- **Rotate the session ID on login, logout, and privilege change** - otherwise a fixed session survives session-fixation attacks.

**JWT** - signed, not encrypted; anyone can decode the payload:

```json
{"sub": "42", "exp": 1735689600, "scope": "read"}
```

- HS256 (shared secret, simpler) vs RS256 (asymmetric - verifiers never hold the signing key; the right choice across services).
- Always set a short `exp` (minutes); use refresh tokens for longevity.
- **Revocation is the weakness**: you cannot un-issue a stateless token. Keep access tokens short-lived and maintain a `jti` denylist (or a version stamp per user) for kill switches.

**OAuth2** - use the authorization code flow **with PKCE** for every client, including SPAs and mobile:

```
GET /authorize?...&code_challenge=S256(verifier)
POST /token     { "code": ..., "code_verifier": verifier }
```

- PKCE prevents authorization-code interception; the implicit flow is deprecated for good reason.
- Request the minimum scopes; the token expiry bounds the blast radius of a leak.

## Authorization Patterns

- **RBAC** (roles map to permissions) for coarse structure; **ABAC** (attributes like "record owner", "during business hours") for nuance. Most systems need both.
- **Resource-based checks are the ones that matter**: not "can edit posts" but "can edit *this* post" - `post.author_id == current_user.id`. Role-only checks are how IDOR happens.
- Enforce at a single **policy enforcement point** (middleware/guard) instead of scattered `if user.role ==` checks that drift apart under change.
- **Least privilege**: every identity - users, service accounts, tokens - gets the minimum scope that works.

## Input and Output Handling

- Validate at the trust boundary: reject anything not matching a strict schema (types, ranges, lengths), server-side always.
- Sanitize with allowlists (`^[a-z0-9-]+$`), never blocklists - a blocklist is a list of attacks you have thought of so far.
- **Encode output for its context** - the HTML escape is wrong inside JavaScript:

```javascript
el.textContent = user.name;              // HTML context: browser handles it safely
// never: el.innerHTML = user.name;
JSON.stringify(user.name);               // JS/JSON context
encodeURIComponent(user.name);           // URL context
```

## Security Headers

```
Content-Security-Policy: default-src 'self'; script-src 'self'   # blocks XSS payload execution
Strict-Transport-Security: max-age=63072000; includeSubDomains   # forces HTTPS from first visit on
X-Content-Type-Options: nosniff                                   # stops MIME-type confusion
X-Frame-Options: DENY                                             # stops clickjacking via iframe
Referrer-Policy: strict-origin-when-cross-origin                  # leaks less URL data to third parties
```

CSP is the highest-leverage header: even an injected script cannot execute or phone home if the policy forbids inline and unknown origins. Roll it out in report-only mode, tighten until quiet, then enforce.

## CORS

The same-origin policy is the browser's default isolation. CORS *relaxes* it - every relaxation is an attack-surface decision:

```
Access-Control-Allow-Origin: https://app.example.com    # never * with credentials
Access-Control-Allow-Credentials: true
Vary: Origin
```

- Preflight (OPTIONS) fires for non-simple requests; answer it correctly or the browser never sends the real request.
- `Access-Control-Allow-Origin: *` combined with `credentials: "include"` is rejected by browsers. Reflecting arbitrary `Origin` values server-side "to make it work" recreates that vulnerability on purpose - any website can then act as the logged-in user.
- Allow only a fixed list of known origins.

## API Security

- **Rate limiting** per key/IP/user - brute force and scraping are volume attacks; make volume expensive (429 + `Retry-After`).
- **API key rotation**: set expiries and run a two-live-keys pattern so rotation is not an outage.
- **Request signing** (HMAC or SigV4-style) when webhooks and machine clients must prove authenticity.
- **Idempotency keys** on POSTs that pay or create - network retries and user double-clicks must not double-charge.
- **Input size limits** (body caps, field-length caps) - a 2 GB JSON body is a DoS vector before it is a parsing problem.

## Common Pitfalls

- String-concatenated queries "just this once" - that one instance is the SQL injection.
- Trusting client-side checks: hidden fields, disabled buttons, or a role claim the server never verifies.
- Secrets in logs, error messages, or git - treat leaked as compromised; rotate.
- Reflecting any `Origin` header in CORS with credentials enabled.
- Storing JWTs in `localStorage` - XSS reads it; prefer HttpOnly cookies or memory plus refresh.
- `innerHTML` with "already sanitized" input - encode at output time, in the right context.
- Skipping security logging until after the incident - you cannot investigate what was never recorded.

## Definition of Done

- [ ] All queries parameterized or via ORM; deserialization of untrusted data eliminated
- [ ] Auth: MFA supported, bcrypt/argon2 hashing, session rotation on login, short-expiry tokens
- [ ] Access control deny-by-default, verified server-side on every request and per resource
- [ ] Output encoded per context; CSP plus the security header set enforced
- [ ] CORS restricted to known origins; no credentialed wildcards or Origin reflection
- [ ] Rate limits, size limits, and idempotency on mutating endpoints; security events logged
