//! The host-side egress boundary a sandboxed command reaches the network
//! through: the domain policy, the forbidden-address check, the loopback
//! proxy-port sweep, and the decision core of the loopback proxy (parity
//! target: `internal/safety/egress`). A refusal carries a typed
//! [`policy::Reason`] the client sees and the host records against the
//! command that caused it. The HTTP mechanics stay with the transport,
//! which wires [`proxy::UpstreamSelector`], [`proxy::EgressApprover`] and
//! a resolver through [`proxy::EgressGate`].

pub mod address;
pub mod loopback;
pub mod policy;
pub mod proxy;

pub use address::forbidden_address;
pub use loopback::{
    SystemProxySource, loopback_proxy_ports, loopback_proxy_ports_with, parse_scutil_proxies,
    set_system_proxy_source,
};
pub use policy::{Policy, Reason};
pub use proxy::{
    Admission, AdmissionError, ApprovalFailure, Denial, ENV_KEYS, EgressApprover, EgressGate,
    EgressRouteHandle, LocalAddresses, LookupError, Resolver, UpstreamFailure, UpstreamProxy,
    UpstreamSelector, env, new_token, proxy_token,
};
