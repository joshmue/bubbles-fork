//! The interface exported launchers call, on the bus name GApplication already
//! owns. A custom interface rather than an `org.freedesktop.Application`
//! action, because the launcher needs the answer: only the calling process
//! knows a human is waiting behind a menu entry, and only it can put the
//! failure in front of them.

use gtk::gio::{self, DBusConnection, DBusMethodInvocation};

use bubbles::{
    agent_request, start_app_path, LAUNCHER_FAILED_ERROR, LAUNCHER_INTERFACE,
    LAUNCHER_NOT_INSTALLED_ERROR, LAUNCHER_NOT_RUNNING_ERROR, LAUNCHER_OBJECT_PATH,
};

use crate::agents;

const INTROSPECTION: &str = r#"
<node>
  <interface name="de.gonicus.Bubbles.Launcher">
    <method name="StartApplication">
      <arg type="s" name="bubble" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
    </method>
  </interface>
</node>
"#;

pub fn register(connection: &DBusConnection) {
    let node = match gio::DBusNodeInfo::for_xml(INTROSPECTION) {
        Ok(node) => node,
        Err(error) => return eprintln!("could not parse the launcher interface: {error}"),
    };
    let Some(interface) = node.lookup_interface(LAUNCHER_INTERFACE) else {
        return;
    };

    let registration = connection
        .register_object(LAUNCHER_OBJECT_PATH, &interface)
        .method_call(|_connection, _sender, _path, _interface, method, parameters, invocation| {
            match method {
                "StartApplication" => {
                    let (bubble, app_id) = parameters.get::<(String, String)>()
                        .expect("the signature we declared");
                    start_application(bubble, app_id, invocation);
                }
                _ => invocation.return_dbus_error(LAUNCHER_FAILED_ERROR, "unknown method"),
            }
        })
        .build();
    if let Err(error) = registration {
        eprintln!("could not export the launcher interface: {error}");
    }
}

fn start_application(bubble: String, app_id: String, invocation: DBusMethodInvocation) {
    let Some(addr) = agents::get(&bubble) else {
        invocation.return_dbus_error(
            LAUNCHER_NOT_RUNNING_ERROR,
            &format!("the bubble “{bubble}” is not running"),
        );
        return;
    };
    relm4::spawn_local(async move {
        match agent_request(addr, hyper::Method::POST, &start_app_path(&app_id)).await {
            Ok(response) if (200..300).contains(&response.status) => invocation.return_value(None),
            // The application was uninstalled inside the bubble since it was
            // exported, which the launcher reports differently.
            Ok(response) if response.status == 404 => invocation.return_dbus_error(
                LAUNCHER_NOT_INSTALLED_ERROR,
                &format!("“{app_id}” is not installed in “{bubble}”"),
            ),
            Ok(response) => {
                let detail = match response.body.trim() {
                    "" => format!("the bubble answered with status {}", response.status),
                    body => body.to_string(),
                };
                invocation.return_dbus_error(LAUNCHER_FAILED_ERROR, &detail);
            }
            Err(error) => invocation.return_dbus_error(LAUNCHER_FAILED_ERROR, &error),
        }
    });
}
