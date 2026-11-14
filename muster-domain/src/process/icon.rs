use strum::{Display, EnumIter, EnumMessage, EnumProperty, EnumString};

/// The mark a process wears in `muster view`. Each kebab-case name is the
/// Simple Icons slug of the brand; `message` is its title and `color` the
/// colour its mark is drawn in on the dark face disc.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    EnumString,
    EnumIter,
    EnumMessage,
    EnumProperty,
)]
#[strum(serialize_all = "kebab-case")]
pub enum ProcessIcon {
    /// Rust.
    #[strum(message = "Rust", props(color = "#dea584"))]
    Rust,
    /// Docker.
    #[strum(message = "Docker", props(color = "#2496ed"))]
    Docker,
    /// Node.js.
    #[strum(message = "Node.js", props(color = "#5fa04e"))]
    Nodedotjs,
    /// Python.
    #[strum(message = "Python", props(color = "#5a9fd4"))]
    Python,
    /// Go.
    #[strum(message = "Go", props(color = "#00add8"))]
    Go,
    /// Vite.
    #[strum(message = "Vite", props(color = "#9499ff"))]
    Vite,
    /// Next.js.
    #[strum(message = "Next.js", props(color = "#f1f1f3"))]
    Nextdotjs,
    /// PostgreSQL.
    #[strum(message = "PostgreSQL", props(color = "#6d8fe6"))]
    Postgresql,
    /// Redis.
    #[strum(message = "Redis", props(color = "#ff4438"))]
    Redis,
    /// MySQL.
    #[strum(message = "MySQL", props(color = "#6fa3d4"))]
    Mysql,
    /// MongoDB.
    #[strum(message = "MongoDB", props(color = "#47a248"))]
    Mongodb,
    /// NGINX.
    #[strum(message = "NGINX", props(color = "#2fbf71"))]
    Nginx,
    /// Deno.
    #[strum(message = "Deno", props(color = "#f1f1f3"))]
    Deno,
    /// Bun.
    #[strum(message = "Bun", props(color = "#fbf0df"))]
    Bun,
    /// Ruby.
    #[strum(message = "Ruby", props(color = "#e0473c"))]
    Ruby,
    /// Ruby on Rails.
    #[strum(message = "Ruby on Rails", props(color = "#e8473b"))]
    Rubyonrails,
    /// Django.
    #[strum(message = "Django", props(color = "#44b78b"))]
    Django,
    /// PHP.
    #[strum(message = "PHP", props(color = "#8f93d6"))]
    Php,
    /// Elixir.
    #[strum(message = "Elixir", props(color = "#a97fd8"))]
    Elixir,
    /// .NET.
    #[strum(message = ".NET", props(color = "#8f7fe8"))]
    Dotnet,
    /// Vue.js.
    #[strum(message = "Vue.js", props(color = "#4fc08d"))]
    Vuedotjs,
    /// Svelte.
    #[strum(message = "Svelte", props(color = "#ff3e00"))]
    Svelte,
    /// Astro.
    #[strum(message = "Astro", props(color = "#d083ff"))]
    Astro,
    /// Nuxt.
    #[strum(message = "Nuxt", props(color = "#00dc82"))]
    Nuxt,
    /// RabbitMQ.
    #[strum(message = "RabbitMQ", props(color = "#ff6600"))]
    Rabbitmq,
    /// A plain terminal, for a process no brand fits.
    #[strum(message = "Terminal", props(color = "#a1a2a8"))]
    Terminal,
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// The color property every variant carries.
    const COLOR: &str = "color";
    /// The length of a `#rrggbb` color.
    const COLOR_LEN: usize = 7;

    /// The names are the Simple Icons slugs, read back by `FromStr`.
    #[test]
    fn names_are_simple_icons_slugs_and_round_trip() {
        assert_eq!(ProcessIcon::Nodedotjs.to_string(), "nodedotjs");
        assert_eq!(ProcessIcon::Nextdotjs.to_string(), "nextdotjs");
        assert_eq!(ProcessIcon::Rubyonrails.to_string(), "rubyonrails");
        for icon in ProcessIcon::iter() {
            assert_eq!(icon.to_string().parse::<ProcessIcon>(), Ok(icon));
        }
        assert!("node.js".parse::<ProcessIcon>().is_err());
        assert!("Rust".parse::<ProcessIcon>().is_err());
    }

    /// Every variant has a title and a `#rrggbb` color.
    #[test]
    fn every_variant_has_a_title_and_a_color() {
        for icon in ProcessIcon::iter() {
            let title = icon.get_message().unwrap_or_default();
            assert!(!title.is_empty(), "{icon} has no title");
            let color = icon.get_str(COLOR).unwrap_or_default();
            assert_eq!(color.len(), COLOR_LEN, "{icon} color {color}");
            assert!(color.starts_with('#'), "{icon} color {color}");
            assert!(
                color[1..].chars().all(|digit| digit.is_ascii_hexdigit()),
                "{icon} color {color}"
            );
        }
        assert_eq!(ProcessIcon::Nodedotjs.get_message(), Some("Node.js"));
        assert_eq!(ProcessIcon::Dotnet.get_message(), Some(".NET"));
    }
}
