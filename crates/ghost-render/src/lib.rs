//! ghost-render: theme engine — Handlebars registry, helpers, layouts.
pub mod apps;
pub mod color;
pub mod context;
pub mod get_helper;
pub mod helpers;
pub mod lexical;
pub mod markdown;
pub mod preprocess;
pub mod social;
pub mod theme;
pub mod util;

pub use helpers::HelperDeps;
pub use theme::{list_themes, load_and_activate, read_pkg, validate, ThemePkg, ThemeState};

#[cfg(test)]
mod probe {
    use handlebars::{
        Context, Handlebars, Helper, HelperDef, HelperResult, Output, RenderContext, Renderable,
    };

    /// Block helper mimicking Ghost's foreach: block params, @index/@first.
    /// Mechanics mirrored from handlebars' built-in each (helper_each.rs):
    /// local vars are stored WITHOUT '@'; block params use BlockParams.
    struct Foreach;

    impl HelperDef for Foreach {
        fn call<'reg: 'rc, 'rc>(
            &self,
            h: &Helper<'rc>,
            r: &'reg Handlebars<'reg>,
            ctx: &'rc Context,
            rc: &mut RenderContext<'reg, 'rc>,
            out: &mut dyn Output,
        ) -> HelperResult {
            let Some(param) = h.param(0) else {
                return Ok(());
            };
            let value = param.value().clone();
            let arr = value.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                if let Some(t) = h.inverse() {
                    t.render(r, ctx, rc, out)?;
                }
                return Ok(());
            }
            let array_path = param.context_path().cloned();
            rc.push_block(handlebars::BlockContext::new());
            for (i, item) in arr.iter().enumerate() {
                if let Some(block) = rc.block_mut() {
                    match &array_path {
                        Some(p) => {
                            let mut newp = p.clone();
                            newp.push(i.to_string());
                            *block.base_path_mut() = newp;
                        }
                        None => block.set_base_value(item.clone()),
                    }
                    block.set_local_var("index", i.into());
                    block.set_local_var("number", (i + 1).into());
                    block.set_local_var("first", (i == 0).into());
                    block.set_local_var("last", (i == arr.len() - 1).into());
                    block.set_local_var("even", (i % 2 == 1).into());
                    block.set_local_var("odd", (i % 2 == 0).into());
                    let mut params = handlebars::BlockParams::new();
                    if let Some((p_key, p_val)) = h.block_param_pair() {
                        params.add_value(p_key, i.into())?;
                        params.add_value(p_val, item.clone())?;
                    } else if let Some(p_val) = h.block_param() {
                        params.add_value(p_val, item.clone())?;
                    }
                    block.set_block_params(params);
                }
                if let Some(t) = h.template() {
                    t.render(r, ctx, rc, out)?;
                }
            }
            rc.pop_block();
            Ok(())
        }
    }

    #[test]
    fn probe_block_helper_mechanics() {
        let mut hb = Handlebars::new();
        hb.register_template_string("t", "{{#foreach items as |item|}}[{{item.title}} i{{@index}} {{#if @first}}F{{/if}}]{{/foreach}}").unwrap();
        hb.register_helper("foreach", Box::new(Foreach));
        let data = serde_json::json!({ "items": [ {"title": "a"}, {"title": "b"} ] });
        let out = hb.render("t", &data).unwrap();
        assert_eq!(out, "[a i0 F][b i1 ]");
    }

    #[test]
    fn probe_partials_and_root() {
        let mut hb = Handlebars::new();
        hb.register_partial("nav", "<nav>{{site.title}}</nav>");
        hb.register_template_string("t", "{{>nav}} {{@root.site.url}}").unwrap();
        let data = serde_json::json!({ "site": {"title": "T", "url": "http://x"} });
        let out = hb.render("t", &data).unwrap();
        assert_eq!(out, "<nav>T</nav> http://x");
    }

    #[test]
    fn probe_else_branch() {
        let mut hb = Handlebars::new();
        hb.register_template_string("t", "{{#foreach items}}x{{else}}empty{{/foreach}}").unwrap();
        hb.register_helper("foreach", Box::new(Foreach));
        let out = hb.render("t", &serde_json::json!({ "items": [] })).unwrap();
        assert_eq!(out, "empty");
    }
}
