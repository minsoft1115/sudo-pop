-- Installed by `sudo-pop --init`. Edit sudo-pop instead of this file.
--
-- Matches the Wayland app-id set in src/gui.rs. no_screen_share keeps
-- the password window out of screen shares and recordings.
--
-- No size rule: sudo-pop asks for a width that fits the command it is about to
-- show (400 to 800), and a rule here would override it.
o.window("^(sudo-askpass)$", {
  float = true,
  center = true,
  dim_around = true,
  stay_focused = true,
  pin = true,
  no_screen_share = true,
})
