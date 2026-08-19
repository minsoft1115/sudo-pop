-- Installed by `sudo-pop --init`. Edit sudo-pop instead of this file.
--
-- Matches the Wayland app-id set in src/gui.rs. no_screen_share keeps
-- the password window out of screen shares and recordings.
o.window("^(sudo-askpass)$", {
  float = true,
  center = true,
  size = { 400, 200 },
  dim_around = true,
  stay_focused = true,
  pin = true,
  no_screen_share = true,
})
