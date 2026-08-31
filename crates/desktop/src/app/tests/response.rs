use super::*;

#[gpui::test]
fn request_editor_sections_render_for_an_open_request(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(request_key, cx);
        })
        .expect("test window should be open");

    for section in EditorSection::ALL {
        window
            .update(cx, |view, _, cx| {
                view.request_editor.section = section;
                if section == EditorSection::Body {
                    view.change_body_kind(request_key, BodyEditorKind::Json, cx);
                }
                cx.notify();
            })
            .expect("test window should remain open");
        cx.run_until_parked();
        {
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            assert!(visual.debug_bounds("request-url-bar").is_some());
            assert!(visual.debug_bounds("request-breadcrumb").is_some());
            assert!(visual.debug_bounds("request-breadcrumb-folder-0").is_some());
            assert!(visual.debug_bounds("request-breadcrumb-request").is_some());
            assert!(visual.debug_bounds("request-method-trigger").is_some());
            assert!(visual.debug_bounds("request-environment-trigger").is_some());
            if section == EditorSection::Body {
                assert!(
                    visual.debug_bounds("request-body-editor").is_some(),
                    "JSON body editor should render"
                );
            }
        }
    }
}

#[gpui::test]
fn completed_response_renders_pretty_raw_headers_and_search(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(request_key, cx);
            let (cancellation, _) = tokio::sync::oneshot::channel();
            let generation = view.execution.begin(request_key, cancellation);
            let body = br#"{"createdAt":1787482800,"ok":true}"#.to_vec();
            view.complete_execution(
                request_key,
                generation,
                Ok(HttpResponse {
                    status: 201,
                    reason: "Created".to_owned(),
                    url: "https://api.example.test/users".to_owned(),
                    duration: Duration::from_millis(42),
                    size: body.len(),
                    headers: vec![ResponseHeader {
                        name: "content-type".to_owned(),
                        value: "application/json".to_owned(),
                    }],
                    body,
                    body_complete: true,
                    body_file: None,
                    body_retention_error: None,
                }),
                cx,
            );
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("response-status").is_some());
        assert!(visual.debug_bounds("response-status-code").is_some());
        assert!(visual.debug_bounds("response-metadata").is_some());
        assert!(visual.debug_bounds("response-tab-pretty").is_some());
        assert!(visual.debug_bounds("response-tab-raw").is_some());
        assert!(visual.debug_bounds("response-tab-headers").is_some());
        assert!(visual.debug_bounds("response-resize-handle").is_some());
        assert!(visual.debug_bounds("sidebar-resize-handle").is_some());
        assert!(visual.debug_bounds("response-raw-view-text").is_none());
        assert!(visual.debug_bounds("response-raw-view-base64").is_none());
        assert!(visual.debug_bounds("response-search").is_none());
        assert!(visual.debug_bounds("editor-search-card").is_none());
        assert!(visual.debug_bounds("response-body").is_some());
        assert!(visual.debug_bounds("response-headers").is_none());
    }

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let body_bounds = visual
            .debug_bounds("response-body")
            .expect("response body should render");
        visual.simulate_click(body_bounds.center(), Modifiers::default());
    }
    cx.simulate_keystrokes(window.into(), find_shortcut());
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("editor-search-card").is_some());
        assert!(visual.debug_bounds("editor-search-input").is_some());
        assert!(visual.debug_bounds("response-search").is_none());
    }
    cx.simulate_keystrokes(window.into(), "o k");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("editor-search-card").is_some());
        assert!(visual.debug_bounds("response-body").is_some());
    }
    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("editor-search-card").is_none());
    }

    window
        .update(cx, |view, _, cx| {
            view.set_response_tab(ResponseViewerTab::Raw, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("response-raw-view-text").is_some());
        let base64 = visual
            .debug_bounds("response-raw-view-base64")
            .expect("raw Base64 sub-tab should render");
        visual.simulate_click(base64.center(), Modifiers::default());
    }
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.response_viewer.raw_view(), RawBodyView::Base64);
        })
        .expect("test window should remain open");
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("response-body").is_some());
    }

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Headers);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("response-headers").is_some());
        assert!(visual.debug_bounds("response-body").is_none());
    }

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Pretty);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Inspect);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("response-inspector-divider").is_some());
        let reveal = visual
            .debug_bounds("response-inspector-reveal-pretty")
            .expect("selected inspection should expose a reveal button");
        visual.simulate_mouse_down(reveal.center(), MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(reveal.center(), MouseButton::Left, Modifiers::default());
    }
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.response_viewer.tab(), ResponseViewerTab::Pretty);
            assert_eq!(
                view.pretty_reveal.get(),
                Some(PrettyRevealState {
                    selection: InspectSelection::Timestamp(0),
                    scroll_pending: false,
                })
            );
        })
        .expect("test window should remain open");
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let body = visual
            .debug_bounds("response-body")
            .expect("Pretty response body should render after reveal");
        assert!(
            visual
                .debug_bounds("response-inspector-reveal-pretty")
                .is_none()
        );
        visual.simulate_mouse_down(body.center(), MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(body.center(), MouseButton::Left, Modifiers::default());
    }
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.pretty_reveal.get().is_none());
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn xml_response_inspects_values_and_keeps_syntax_after_visiting_raw(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(request_key, cx);
            let (cancellation, _) = tokio::sync::oneshot::channel();
            let generation = view.execution.begin(request_key, cancellation);
            let body = br#"<root createdAt="1787482800"><item/></root>"#.to_vec();
            view.complete_execution(
                request_key,
                generation,
                Ok(HttpResponse {
                    status: 200,
                    reason: "OK".to_owned(),
                    url: "https://api.example.test/data.xml".to_owned(),
                    duration: Duration::from_millis(12),
                    size: body.len(),
                    headers: vec![ResponseHeader {
                        name: "content-type".to_owned(),
                        value: "application/xml".to_owned(),
                    }],
                    body,
                    body_complete: true,
                    body_file: None,
                    body_retention_error: None,
                }),
                cx,
            );
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            assert_eq!(
                view.response_viewer
                    .document(request_key)
                    .expect("response document")
                    .syntax
                    .language(),
                "xml"
            );
            let document = view
                .response_viewer
                .document(request_key)
                .expect("response document");
            assert_eq!(document.inspection.timestamps.len(), 1);
            assert_eq!(document.inspection.timestamps[0].path, "/root/@createdAt");
            assert_eq!(document.inspection_ranges.len(), 1);
            assert_eq!(
                &document.pretty_text[document.inspection_ranges[0].range.clone()],
                "1787482800"
            );
            view.response_viewer.set_tab(ResponseViewerTab::Raw);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Pretty);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert_eq!(view.response_viewer.tab(), ResponseViewerTab::Pretty);
            assert_eq!(
                view.response_viewer
                    .document(request_key)
                    .expect("response document")
                    .syntax
                    .language(),
                "xml"
            );
        })
        .expect("test window should remain open");
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("response-body").is_some());
}

#[gpui::test]
fn large_response_body_only_renders_visible_rows(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    let body = (0..20_000)
        .map(|index| format!("line-{index:05}"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(request_key, cx);
            view.shell.response_height = 220.0;
            let (cancellation, _) = tokio::sync::oneshot::channel();
            let generation = view.execution.begin(request_key, cancellation);
            view.complete_execution(
                request_key,
                generation,
                Ok(HttpResponse {
                    status: 200,
                    reason: "OK".to_owned(),
                    url: "https://api.example.test/lines".to_owned(),
                    duration: Duration::from_millis(12),
                    size: body.len(),
                    headers: vec![ResponseHeader {
                        name: "content-type".to_owned(),
                        value: "text/plain".to_owned(),
                    }],
                    body,
                    body_complete: true,
                    body_file: None,
                    body_retention_error: None,
                }),
                cx,
            );
            view.response_viewer.set_tab(ResponseViewerTab::Raw);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.simulate_next_frame(cx);
        });
    }
    cx.run_until_parked();

    let (total_rows, rendered_rows) = window
        .update(cx, |view, _, _| {
            (
                view.response_viewer.visible_line_count(request_key),
                view.rendered_response_rows,
            )
        })
        .expect("test window should remain open");
    assert!(total_rows >= 20_000);
    assert!(rendered_rows > 0);
    assert!(
        rendered_rows < total_rows,
        "virtualized response viewer rendered all {total_rows} rows"
    );
}
