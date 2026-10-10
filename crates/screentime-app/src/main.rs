//! Screen Time as a native GNOME app: a libadwaita window around the dashboard that
//! screentimed serves on localhost. It follows the system light/dark style, is single
//! instance (launching it again raises the window), and opens outside links in the browser.
//!
//! `screentime-app [day|week|month]` opens straight into that view.

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use webkit6::prelude::*;

const APP_ID: &str = "dev.zeroxshubhs.Screentime";

fn base_url() -> String {
    let listen = std::env::var("SCREENTIME_LISTEN").unwrap_or_else(|_| "127.0.0.1:47821".into());
    format!("http://{listen}")
}

/// Dashboard URL for an optional view name passed on the command line.
fn page_url(view: Option<&str>) -> String {
    let hash = match view {
        Some(v @ ("day" | "week" | "month")) => format!("#/{v}/{}", glib::DateTime::now_local().map(|d| d.format("%Y-%m-%d").unwrap().to_string()).unwrap_or_default()),
        _ => String::new(),
    };
    format!("{}/?app=1{hash}", base_url())
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_command_line(|app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).filter_map(|a| a.to_str().map(str::to_string)).collect();
        let url = page_url(args.first().map(String::as_str));
        match app.active_window().and_then(|w| w.downcast::<adw::ApplicationWindow>().ok()) {
            Some(win) => {
                if !args.is_empty() {
                    if let Some(web) = find_web_view(&win) {
                        web.load_uri(&url);
                    }
                }
                win.present();
            }
            None => build_window(app, &url),
        }
        glib::ExitCode::SUCCESS
    });
    app.set_accels_for_action("win.reload", &["<Ctrl>r", "F5"]);
    app.set_accels_for_action("window.close", &["<Ctrl>w"]);
    app.set_accels_for_action("app.quit", &["<Ctrl>q"]);
    let quit = gio::ActionEntry::builder("quit").activate(|app: &adw::Application, _, _| app.quit()).build();
    app.add_action_entries([quit]);
    app.run()
}

fn find_web_view(win: &adw::ApplicationWindow) -> Option<webkit6::WebView> {
    // Window content is ToolbarView > Stack > WebView ("web" page).
    let toolbar = win.content()?.downcast::<adw::ToolbarView>().ok()?;
    let stack = toolbar.content()?.downcast::<gtk::Stack>().ok()?;
    stack.child_by_name("web")?.downcast::<webkit6::WebView>().ok()
}

fn build_window(app: &adw::Application, url: &str) {
    install_css();

    let web = webkit6::WebView::new();
    web.set_vexpand(true);
    // Avoids a white flash before the page paints in dark mode.
    web.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&web) {
        settings.set_enable_developer_extras(std::env::var_os("SCREENTIME_DEVTOOLS").is_some());
        settings.set_enable_back_forward_navigation_gestures(false);
    }

    // Shown when screentimed is not reachable.
    let start = gtk::Button::builder().label("Start Screen Time").halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();
    let down = adw::StatusPage::builder()
        .icon_name("screentime")
        .title("Screen Time isn't running")
        .description("The background service that records your usage is stopped.")
        .child(&start)
        .build();

    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
    stack.add_named(&web, Some("web"));
    stack.add_named(&down, Some("down"));

    let header = adw::HeaderBar::builder().css_classes(["flat", "screentime-header"]).build();
    header.set_title_widget(Some(&adw::WindowTitle::new("Screen Time", "")));
    let reload = gtk::Button::builder().icon_name("view-refresh-symbolic").tooltip_text("Refresh").action_name("win.reload").build();
    header.pack_end(&reload);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    let win = adw::ApplicationWindow::builder()
        .application(app)
        .title("Screen Time")
        .icon_name("screentime")
        .default_width(1040)
        .default_height(900)
        .width_request(360)
        .height_request(480)
        .content(&toolbar)
        .build();

    let reload_action = gio::ActionEntry::builder("reload")
        .activate(glib::clone!(#[weak] web, #[weak] stack, move |_: &adw::ApplicationWindow, _, _| {
            stack.set_visible_child_name("web");
            web.reload();
        }))
        .build();
    win.add_action_entries([reload_action]);

    start.connect_clicked(glib::clone!(#[weak] web, #[weak] stack, move |_| {
        let _ = std::process::Command::new("systemctl").args(["--user", "start", "screentimed.service"]).status();
        glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
            stack.set_visible_child_name("web");
            web.reload();
        });
    }));

    web.connect_load_failed(glib::clone!(#[weak] stack, #[upgrade_or] false, move |_, _, _, _| {
        stack.set_visible_child_name("down");
        true
    }));

    // Pass the system style to the page so it matches the window exactly.
    let style = adw::StyleManager::default();
    let push_theme = glib::clone!(#[weak] web, move |dark: bool| {
        let js = format!("window.setTheme && window.setTheme('{}')", if dark { "dark" } else { "light" });
        web.evaluate_javascript(&js, None, None, None::<&gio::Cancellable>, |_| {});
    });
    web.connect_load_changed(glib::clone!(#[strong] push_theme, #[weak] style, move |_, ev| {
        if ev == webkit6::LoadEvent::Finished {
            push_theme(style.is_dark());
        }
    }));
    style.connect_dark_notify(move |s| push_theme(s.is_dark()));

    // Links that leave the dashboard open in the default browser, not inside the app.
    let base = base_url();
    web.connect_decide_policy(move |_, decision, kind| {
        if !matches!(kind, webkit6::PolicyDecisionType::NavigationAction | webkit6::PolicyDecisionType::NewWindowAction) {
            return false;
        }
        let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else { return false };
        let uri = nav.navigation_action().and_then(|a| a.request()).and_then(|r| r.uri()).map(|u| u.to_string());
        match uri {
            Some(u) if !u.starts_with(&base) && !u.starts_with("about:") => {
                let _ = gio::AppInfo::launch_default_for_uri(&u, None::<&gio::AppLaunchContext>);
                decision.ignore();
                true
            }
            _ => false,
        }
    });

    web.load_uri(url);
    win.present();
}

/// Header bar in the dashboard's background color, so window and page read as one surface.
fn install_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".screentime-header { background: #f2f2f7; box-shadow: none; }
         @media (prefers-color-scheme: dark) { .screentime-header { background: #000000; } }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
