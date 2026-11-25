/*
 * C side of the connection guard (see tls_guard.rs).
 *
 * pgrx does not expose PostgreSQL's client authentication hook nor the layout
 * of the Port structure. Rather than re-declaring them by hand in Rust, these
 * few accessors are compiled against the real PostgreSQL headers, so they
 * always match the server the extension is built for.
 */
#include "postgres.h"

#include <sys/socket.h>

#include "libpq/auth.h"
#include "libpq/libpq-be.h"

/* Did the client connect through the Unix socket? */
bool
pgxila_port_is_unix_socket(const Port *port)
{
	return port->raddr.addr.ss_family == AF_UNIX;
}

/*
 * TLS protocol version of the connection ("TLSv1.3"), or NULL when the
 * connection is not encrypted with TLS.
 */
const char *
pgxila_port_tls_version(Port *port)
{
#ifdef USE_SSL
	if (port->ssl_in_use)
		return be_tls_get_version(port);
#endif
	return NULL;
}

/* Installs a client authentication hook and returns the previous one. */
ClientAuthentication_hook_type
pgxila_swap_client_authentication_hook(ClientAuthentication_hook_type hook)
{
	ClientAuthentication_hook_type previous = ClientAuthentication_hook;

	ClientAuthentication_hook = hook;
	return previous;
}
