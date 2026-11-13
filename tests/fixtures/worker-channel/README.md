# Worker TLS fixture

Public test CA, server certificate and **test-only private key**, generated with
OpenSSL for controller.tail123.ts.net and worker.tail123.ts.net. No real node,
account, network, credentials or public trust chain is involved. The CA private
key is discarded. Production transports never load these roots; only cfg(test)
constructors can supply the fixture CA. Certificate validation remains enabled.
