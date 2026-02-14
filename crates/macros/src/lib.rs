use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, LitStr, parse::Parse, parse::ParseStream, Token, Ident, ExprArray, Expr};
use base64::{Engine as _, engine::general_purpose};
use std::path::PathBuf;

/// Parsed plugin attribute structure
struct PluginAttr {
    name: String,
    category: String, // Added: category information
    service_type: String,
    description: String,
    version: String,
    icon: Option<String>,
    action_icon: Option<String>,
    config_keys: Vec<String>,
    permissions: Vec<String>,
    capabilities: Vec<String>,
    tags: Vec<String>,
    tools: Vec<String>, // M-12: provided_tools support
}

impl Parse for PluginAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = String::new();
        let mut category = String::new(); // Added
        let mut service_type = String::new();
        let mut description = String::new();
        let mut version = String::from("0.1.0");
        let mut icon = None;
        let mut action_icon = None;
        let mut config_keys = Vec::new();
        let mut permissions = Vec::new();
        let mut capabilities = Vec::new();
        let mut tags = Vec::new();
        let mut tools = Vec::new();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            
            if input.peek(LitStr) {
                let val: LitStr = input.parse()?;
                match key.to_string().as_str() {
                    "name" => name = val.value(),
                    "category" => category = val.value(), // Added
                    "kind" => service_type = val.value(),
                    "description" => description = val.value(),
                    "version" => version = val.value(),
                    "icon" => icon = Some(val.value()),
                    "action_icon" => action_icon = Some(val.value()),
                    _ => {}
                }
            } else if input.peek(syn::token::Bracket) {
                let content: ExprArray = input.parse()?;
                let vals: Vec<String> = content.elems.iter().filter_map(|e| {
                    if let Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = e {
                        Some(s.value())
                    } else {
                        None
                    }
                }).collect();

                match key.to_string().as_str() {
                    "permissions" => permissions = vals,
                    "capabilities" => capabilities = vals,
                    "config_keys" => config_keys = vals,
                    "tags" => tags = vals,
                    "tools" => tools = vals, // M-12
                    _ => {}
                }
            }
            
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(PluginAttr { name, category, service_type, description, version, icon, action_icon, config_keys, permissions, capabilities, tags, tools })
    }
}

/// Main macro entry point
#[proc_macro_attribute]
pub fn exiv_plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let attr = parse_macro_input!(attr as PluginAttr);
    
    match emit_plugin_code(input, attr) {
        Ok(expanded) => TokenStream::from(expanded),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Code generation logic
fn emit_plugin_code(input: DeriveInput, attr: PluginAttr) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    // Early validation: check required fields (reduces compilation time on errors)
    if attr.name.is_empty() {
        return Err(syn::Error::new_spanned(&input.ident, "Plugin 'name' is required"));
    }
    if attr.service_type.is_empty() {
        return Err(syn::Error::new_spanned(&input.ident, "Plugin 'kind' (service_type) is required"));
    }
    if attr.description.is_empty() {
        return Err(syn::Error::new_spanned(&input.ident, "Plugin 'description' is required"));
    }

    let factory_name = quote::format_ident!("{}Factory", name);
    let plugin_name_str = &attr.name;
    let service_type_ident = quote::format_ident!("{}", attr.service_type);
    let description_str = &attr.description;
    let version_str = &attr.version;
    
    let action_icon_token = match &attr.action_icon {
        Some(i) => quote! { Some(#i.to_string()) },
        None => quote! { None },
    };
    let config_keys_tokens = attr.config_keys.iter().map(|k| quote! { #k.to_string() });
    // M-12: Generate provided_tools tokens
    let tools_tokens = attr.tools.iter().map(|t| quote! { #t.to_string() });

    // Icon embedding process
    // Optimization: EXIV_SKIP_ICON_EMBED=1 skips icon embedding (faster development builds)
    let icon_data_tokens = if let Some(ref icon_path_str) = attr.icon {
        let skip_embed = std::env::var("EXIV_SKIP_ICON_EMBED")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        if skip_embed {
            // Development mode: skip icon embedding (reduces build time)
            eprintln!("⚡ Skipping icon embed for {} (EXIV_SKIP_ICON_EMBED=1)", plugin_name_str);
            quote! { None }
        } else {
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
            let mut icon_path = PathBuf::from(manifest_dir);
            icon_path.push(icon_path_str);

            match std::fs::read(&icon_path) {
                Ok(bytes) => {
                    let byte_count = bytes.len();
                    if byte_count > 64 * 1024 {
                        return Err(syn::Error::new_spanned(
                            &input.ident,
                            format!("🔌 Plugin Icon '{}' is too large ({} bytes). Limit is 64KB.", icon_path_str, byte_count)
                        ));
                    }
                    let base64_data = general_purpose::STANDARD.encode(bytes);
                    eprintln!("🔌 Embedded icon for {} ({} bytes)", plugin_name_str, byte_count);
                    quote! { Some(#base64_data.to_string()) }
                }
                Err(e) => {
                    return Err(syn::Error::new_spanned(
                        &input.ident,
                        format!("🔌 Failed to read icon at '{}': {}", icon_path.display(), e)
                    ));
                }
            }
        }
    } else {
        quote! { None }
    };

    let perms = attr.permissions.iter().map(|p| {
        let ident = quote::format_ident!("{}", p);
        quote! { exiv_shared::Permission::#ident }
    });

    let caps = attr.capabilities.iter().map(|c| {
        let ident = quote::format_ident!("{}", c);
        quote! { exiv_shared::CapabilityType::#ident }
    });

    let mut default_tags = match attr.service_type.as_str() {
        "Reasoning" => vec!["#MIND", "#LLM"],
        "Memory" => vec!["#MEMORY"],
        "Skill" | "Action" => vec!["#TOOL"],
        "Vision" => vec!["#VISION", "#SENSOR"],
        "HAL" => vec!["#HAL"],
        "Communication" => vec!["#ADAPTER"],
        _ => vec![],
    };

    // Merge user-specified tags
    let user_tags: Vec<&str> = attr.tags.iter().map(|s| s.as_str()).collect();
    default_tags.extend(user_tags);

    let tags_tokens = default_tags.iter().map(|t| quote! { #t.to_string() });

    // Generate downcast implementations based on capabilities
    let mut cast_methods = quote! {};
    for cap in &attr.capabilities {
        match cap.as_str() {
            "Reasoning" => cast_methods.extend(quote! { fn as_reasoning(&self) -> Option<&dyn exiv_shared::ReasoningEngine> { Some(self) } }),
            "Memory" => cast_methods.extend(quote! { fn as_memory(&self) -> Option<&dyn exiv_shared::MemoryProvider> { Some(self) } }),
            "Communication" => cast_methods.extend(quote! { fn as_communication(&self) -> Option<&dyn exiv_shared::CommunicationAdapter> { Some(self) } }),
            "Tool" => cast_methods.extend(quote! { fn as_tool(&self) -> Option<&dyn exiv_shared::Tool> { Some(self) } }),
            "Web" => cast_methods.extend(quote! { fn as_web(&self) -> Option<&dyn exiv_shared::WebPlugin> { Some(self) } }),
            _ => {}
        }
    }

    let category_ident = if attr.category.is_empty() {
        // L-04: Inference logic with complete coverage
        match attr.service_type.as_str() {
            "Reasoning" => quote! { exiv_shared::PluginCategory::Agent },
            "Memory" => quote! { exiv_shared::PluginCategory::Memory },
            "Tool" | "HAL" | "Communication" | "Vision" | "Skill" | "Action" => quote! { exiv_shared::PluginCategory::Tool },
            _ => quote! { exiv_shared::PluginCategory::Other },
        }
    } else {
        let cat = quote::format_ident!("{}", attr.category);
        quote! { exiv_shared::PluginCategory::#cat }
    };

    Ok(quote! {
        #input

        impl #name {
            pub const PLUGIN_ID: &'static str = #plugin_name_str;

            pub fn factory() -> std::sync::Arc<dyn exiv_shared::PluginFactory> {
                std::sync::Arc::new(#factory_name)
            }

            fn auto_manifest(&self) -> exiv_shared::PluginManifest {
                exiv_shared::PluginManifest {
                    id: Self::PLUGIN_ID.to_string(),
                    name: #plugin_name_str.to_string(),
                    description: #description_str.to_string(),
                    version: #version_str.to_string(),
                    category: #category_ident,
                    service_type: exiv_shared::ServiceType::#service_type_ident,
                    tags: vec![ #(#tags_tokens),* ],
                    is_active: true,
                    is_configured: true,
                    required_config_keys: vec![ #(#config_keys_tokens),* ],
                    action_icon: #action_icon_token,
                    action_target: None,
                    icon_data: #icon_data_tokens,
                    magic_seal: 0x56455253, // VERS - must match kernel validation
                    // M-14: Use exiv_shared::SDK_VERSION for consistent version reporting
                    sdk_version: exiv_shared::SDK_VERSION.to_string(),
                    required_permissions: vec![ #(#perms),* ],
                    provided_capabilities: vec![ #(#caps),* ],
                    // M-12: Support tools attribute from macro
                    provided_tools: vec![ #(#tools_tokens),* ],
                }
            }
        }

        impl exiv_shared::PluginCast for #name {
            fn as_any(&self) -> &dyn std::any::Any { self }
            #cast_methods
        }

        pub struct #factory_name;

        #[async_trait::async_trait]
        impl exiv_shared::PluginFactory for #factory_name {
            fn name(&self) -> &str { #plugin_name_str }
            fn service_type(&self) -> exiv_shared::ServiceType {
                exiv_shared::ServiceType::#service_type_ident
            }
            async fn create(&self, config: exiv_shared::PluginConfig) -> anyhow::Result<std::sync::Arc<dyn exiv_shared::Plugin>> {
                let plugin = #name::new_plugin(config).await?;
                Ok(std::sync::Arc::new(plugin))
            }
        }

        exiv_shared::inventory::submit! {
            exiv_shared::PluginRegistrar {
                factory: #name::factory,
            }
        }
    })
}