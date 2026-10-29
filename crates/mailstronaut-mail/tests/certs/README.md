These are public, synthetic loopback TLS fixtures. `localhost-key.pem` is
intentionally public test material, not a production credential. The leaf
certificate is signed by `ca.pem`, has only DNS `localhost`, and expires in 2036.
The CA signing key is not included. No fixture certificate is trusted by default
by the application or installed in a system trust store.

The tests supply this CA explicitly to prove successful TLS, then omit it or
connect using an unmatched IP hostname to prove authentication is withheld.
`expired.pem` uses the same leaf key and CA, with a fixed 2020 validity interval,
to test expiry independently of the untrusted-root and hostname controls.
Trust revocation during an already established session remains untested.
