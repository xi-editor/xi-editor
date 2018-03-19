mod view;

use std::collections::HashMap;

use xi_core::{ViewIdentifier, PluginPid, plugin_rpc};
use xi_rpc::{self, RpcLoop, RpcCtx, RemoteError, ReadError, Handler as RpcHandler};
use self::view::{Plugin, View};

/// Handles raw RPCs from core, updating documents and bridging calls
/// to the plugin,
pub struct Dispatcher<'a, P: 'a + Plugin> {
    //TODO: when we add multi-view, this should be an Arc+Mutex/Rc+RefCell
    views: HashMap<ViewIdentifier, View<P::Cache>>,
    pid: PluginPid,
    plugin: &'a mut P,
}

