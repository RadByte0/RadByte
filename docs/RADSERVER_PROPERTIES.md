# RadServer properties and Paper compatibility

RadServer reads `rad.properties` in its working directory. It is not Paper and
does not implement all options in Paper's `server.properties`. Unknown keys
are rejected at startup so that unsupported security options cannot appear to
work. Keep the existing file and world when upgrading.

The following Paper property names are supported: `motd`, `max-players`,
`view-distance`, `gamemode`, `difficulty`, `level-name`, `force-gamemode`,
`white-list`, `enforce-whitelist`, `op-permission-level`, `spawn-protection`,
`enable-status`, and `hide-online-players`. `game-mode` remains accepted as an
older RadServer spelling; do not specify both. Game modes are `survival`,
`creative`, `adventure`, and `spectator`; difficulties are `peaceful`, `easy`,
`normal`, and `hard`.

`server-ip`, `server-port`, and numeric `level-seed` use Paper's property names.
The older `bind-addr` and `world-seed` spellings remain accepted, but must not be
specified alongside their replacements. RadServer's `world-type` configures its
own generator, not Paper's vanilla generator; the same seed does not produce
the same terrain. Changing generation settings does not regenerate an existing
world. RadServer-specific options are in `config/radserver-extra.properties`.
The old location beside `rad.properties` remains supported; if both files
exist, startup fails until the duplicate is resolved. Legacy configurations
containing these settings in `rad.properties` also remain supported.

Paper/Bukkit analogues include `ticks-per.autosave` in `bukkit.yml`, world
spawn limits, and `proxies.velocity` in `config/paper-global.yml`. These are
not identical to RadServer's seconds-based autosave, global mob cap and
forwarding modes; the RadServer file does not use Paper's YAML format.

Network timeouts in `config/radserver-extra.properties` are expressed in
seconds and must be between 1 and 300: `login-timeout-secs` (default 10 per
login packet), `forwarding-timeout-secs` (5), `configuration-timeout-secs`
(10 total for configuration), and `play-read-timeout-secs` (35 between play
packets). Other keepalive and connection deadlines still apply independently.

Not implemented as Paper properties include `online-mode`, secure profile
enforcement, `allow-flight`, `simulation-distance`, RCON, query, resource packs,
management server, datapack startup selection, watchdog, and Paper's other
properties. In particular, do not assume a direct RadServer connection
authenticates a player's Microsoft account. Enabling equivalent functionality
requires implementation and testing, not merely adding a configuration line.
