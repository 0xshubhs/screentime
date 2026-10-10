//! Screen Time as a native GNOME app: a libadwaita window around the dashboard that
//! screentimed serves on localhost. It follows the system light/dark style, is single
//! instance (launching it again raises the window), and opens outside links in the browser.
//!
//! `screentime-app [day|week|month]` opens straight into that view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use serde::Deserialize;
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

    let (header, controls) = build_header(&web);

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
        .css_classes(["screentime-window"])
        .build();

    // The page reports its view and date after every render; mirror them in the header.
    let ucm = web.user_content_manager().expect("web view has a content manager");
    ucm.register_script_message_handler("screentime", None);
    ucm.connect_script_message_received(Some("screentime"), move |_, value| {
        if let Ok(state) = serde_json::from_str::<PageState>(&value.to_str()) {
            controls.sync(&state);
        }
    });

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

    // Pass the system style to the page, and paint the window to match it, so the header
    // and the page read as one surface in both styles.
    let style = adw::StyleManager::default();
    let push_theme = glib::clone!(#[weak] web, #[weak] win, move |dark: bool| {
        if dark {
            win.add_css_class("dark");
        } else {
            win.remove_css_class("dark");
        }
        run_js(&web, &format!("window.setTheme && window.setTheme('{}')", if dark { "dark" } else { "light" }));
    });
    push_theme(style.is_dark());
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

fn run_js(web: &webkit6::WebView, code: &str) {
    web.evaluate_javascript(code, None, None, None::<&gio::Cancellable>, |_| {});
}

/// What the page posts after each render.
#[derive(Deserialize)]
struct PageState {
    view: String,
    anchor: String,
    title: String,
    can_next: bool,
    is_current: bool,
}

/// Header widgets that mirror the page's state.
struct Controls {
    views: [(&'static str, gtk::ToggleButton); 3],
    date: gtk::MenuButton,
    calendar: gtk::Calendar,
    next: gtk::Button,
    today: gtk::Button,
    view: RefCell<String>,
    /// True while applying page state, so the widgets' own signals do not echo back.
    syncing: Cell<bool>,
}

impl Controls {
    fn sync(&self, s: &PageState) {
        self.syncing.set(true);
        for (name, button) in &self.views {
            button.set_active(*name == s.view);
        }
        self.date.set_label(&s.title);
        self.next.set_sensitive(s.can_next);
        self.today.set_visible(!s.is_current);
        let mut parts = s.anchor.split('-').filter_map(|p| p.parse::<i32>().ok());
        if let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) {
            if let Ok(dt) = glib::DateTime::from_local(y, m, d, 12, 0, 0.0) {
                #[allow(deprecated)]
                self.calendar.select_day(&dt);
            }
        }
        *self.view.borrow_mut() = s.view.clone();
        self.syncing.set(false);
    }
}

/// Header bar holding the Day/Week/Month switcher and date navigation, like Screen Time's
/// toolbar. The page itself hides its own top bar inside the app.
fn build_header(web: &webkit6::WebView) -> (adw::HeaderBar, Rc<Controls>) {
    let header = adw::HeaderBar::builder().css_classes(["flat"]).build();

    let switcher = gtk::Box::builder().css_classes(["linked", "view-switcher"]).build();
    let make = |label: &str| gtk::ToggleButton::builder().label(label).width_request(76).build();
    let (day, week, month) = (make("Day"), make("Week"), make("Month"));
    week.set_group(Some(&day));
    month.set_group(Some(&day));
    for b in [&day, &week, &month] {
        switcher.append(b);
    }
    header.set_title_widget(Some(&switcher));

    let prev = gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text("Previous").build();
    let next = gtk::Button::builder().icon_name("go-next-symbolic").tooltip_text("Next").build();
    let calendar = gtk::Calendar::new();
    let popover = gtk::Popover::builder().child(&calendar).build();
    let date = gtk::MenuButton::builder().label("Today").popover(&popover).css_classes(["flat", "date-button"]).build();
    let nav = gtk::Box::builder().spacing(2).build();
    nav.append(&prev);
    nav.append(&date);
    nav.append(&next);
    header.pack_start(&nav);

    let today = gtk::Button::builder().label("Today").visible(false).build();
    let reload = gtk::Button::builder().icon_name("view-refresh-symbolic").tooltip_text("Refresh (Ctrl+R)").action_name("win.reload").build();
    header.pack_end(&reload);
    header.pack_end(&today);

    let controls = Rc::new(Controls {
        views: [("day", day), ("week", week), ("month", month)],
        date,
        calendar: calendar.clone(),
        next: next.clone(),
        today: today.clone(),
        view: RefCell::new("day".into()),
        syncing: Cell::new(false),
    });

    for (name, button) in &controls.views {
        let name = *name;
        button.connect_toggled(glib::clone!(#[weak] web, #[weak] controls, move |b| {
            if b.is_active() && !controls.syncing.get() {
                run_js(&web, &format!("screentime.go('{name}')"));
            }
        }));
    }
    prev.connect_clicked(glib::clone!(#[weak] web, move |_| run_js(&web, "screentime.step(-1)")));
    next.connect_clicked(glib::clone!(#[weak] web, move |_| run_js(&web, "screentime.step(1)")));
    today.connect_clicked(glib::clone!(#[weak] web, move |_| run_js(&web, "screentime.today()")));
    calendar.connect_day_selected(glib::clone!(#[weak] web, #[weak] controls, #[weak] popover, move |cal| {
        if controls.syncing.get() {
            return;
        }
        let picked = cal.date().format("%Y-%m-%d").map(|d| d.to_string()).unwrap_or_default();
        run_js(&web, &format!("screentime.go('{}', '{picked}')", controls.view.borrow()));
        popover.popdown();
    }));

    (header, controls)
}

/// Window background in the dashboard's colors, so the flat header bar and the page are one
/// surface. The "dark" class follows libadwaita's style manager.
fn install_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        "window.screentime-window, window.screentime-window headerbar { background: #f2f2f7; box-shadow: none; }
         window.screentime-window.dark, window.screentime-window.dark headerbar { background: #000000; }
         .date-button label { font-weight: bold; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
