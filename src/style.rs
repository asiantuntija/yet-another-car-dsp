pub const APP_CSS: &str = "
    /* Hint: Use @theme_ variables (e.g., @theme_accent_color, @theme_bg_color) for GNOME theme integration */
    sidebar {
        background-color: @theme_view_bg_color;
        padding: 12px;
    }
    .sidebar scrolledwindow {
        border: none;
        box-shadow: none;
        background-color: transparent;
    }
    .sidebar button {
        padding: 12px 10px;
        margin-bottom: 8px;
        min-height: 48px;
    }
    .project-title {
        font-weight: bold;
        font-size: 18px;
        margin-bottom: 20px;
    }
    .suggested-action {
        font-weight: bold;
        border-radius: 6px;
    }
    .destructive-action {
        font-weight: bold;
        border-radius: 6px;
    }
    .dirty-button {
        border-left: 5px solid #f90935;
    }
    .dirty-indicator {
        font-size: 10px;
        color: @theme_bg_color;
        opacity: 0.6;
        margin-top: 2px;
    }
    .freq-value {
        font-size: 16px;
        font-weight: bold;
        margin-bottom: 12px;
    }
    .actions-box {
        background-color: @theme_bg_color;
        padding: 10px;
        border-radius: 8px;
    }
";

