//! Client-facing multi-language localization (`ClientInformation` locale + `/lang` command)
//! shared across `RadProxy`, `RadLimbo`, and `RadServer`.

use crate::types::Decode;

/// Supported Minecraft client languages for command responses and player-facing notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientLang {
    /// English (`en_us`, default fallback).
    #[default]
    En,
    /// Ukrainian (`uk_ua`).
    Uk,
    /// Russian (`ru_ru`).
    Ru,
    /// Polish (`pl_pl`).
    Pl,
    /// German (`de_de`).
    De,
    /// French (`fr_fr`).
    Fr,
    /// Spanish (`es_es`, `es_mx`, etc.).
    Es,
    /// Portuguese (`pt_br`, `pt_pt`).
    Pt,
    /// Italian (`it_it`).
    It,
    /// Turkish (`tr_tr`).
    Tr,
    /// Dutch (`nl_nl`).
    Nl,
    /// Chinese (`zh_cn`, `zh_tw`).
    Zh,
    /// Japanese (`ja_jp`).
    Ja,
    /// Korean (`ko_kr`).
    Ko,
}

impl ClientLang {
    /// Maps a Minecraft client locale string (e.g. `"uk_ua"`, `"en_US"`) to [`ClientLang`].
    #[must_use]
    pub fn from_locale(locale: &str) -> Self {
        Self::from_code(locale).unwrap_or(Self::En)
    }

    /// Parses a short language code (`"uk"`, `"en"`, `"uk_ua"`) into [`ClientLang`].
    #[must_use]
    pub fn from_code(raw: &str) -> Option<Self> {
        let norm = raw.trim().to_ascii_lowercase();
        let prefix = norm.split(['_', '-']).next().unwrap_or("");
        match prefix {
            "en" => Some(Self::En),
            "uk" => Some(Self::Uk),
            "ru" => Some(Self::Ru),
            "pl" => Some(Self::Pl),
            "de" => Some(Self::De),
            "fr" => Some(Self::Fr),
            "es" => Some(Self::Es),
            "pt" => Some(Self::Pt),
            "it" => Some(Self::It),
            "tr" => Some(Self::Tr),
            "nl" => Some(Self::Nl),
            "zh" => Some(Self::Zh),
            "ja" => Some(Self::Ja),
            "ko" => Some(Self::Ko),
            _ => None,
        }
    }

    /// Returns the ISO 639-1 language code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Uk => "uk",
            Self::Ru => "ru",
            Self::Pl => "pl",
            Self::De => "de",
            Self::Fr => "fr",
            Self::Es => "es",
            Self::Pt => "pt",
            Self::It => "it",
            Self::Tr => "tr",
            Self::Nl => "nl",
            Self::Zh => "zh",
            Self::Ja => "ja",
            Self::Ko => "ko",
        }
    }

    /// Returns the native display name of this language.
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Uk => "Українська",
            Self::Ru => "Русский",
            Self::Pl => "Polski",
            Self::De => "Deutsch",
            Self::Fr => "Français",
            Self::Es => "Español",
            Self::Pt => "Português",
            Self::It => "Italiano",
            Self::Tr => "Türkçe",
            Self::Nl => "Nederlands",
            Self::Zh => "中文",
            Self::Ja => "日本語",
            Self::Ko => "한국어",
        }
    }
}

/// Decodes the `locale` field from a Minecraft `ClientInformation` packet payload.
#[must_use]
pub fn decode_client_locale(payload: &[u8]) -> Option<String> {
    let mut slice = payload;
    String::decode(&mut slice).ok()
}

/// Client-facing translatable messages for `/lang` and launcher commands.
pub enum ClientMsg<'a> {
    /// Current language status.
    LangCurrent(ClientLang),
    /// Language updated confirmation.
    LangSet(ClientLang),
    /// Language reset to client locale.
    LangAuto(ClientLang),
    /// `/lang` usage hint.
    LangUsage,
    /// No permission to run a command.
    NoPermission(&'a str),
    /// Player is not online.
    PlayerNotOnline(&'a str),
    /// Player does not have permission to join a restricted server.
    NoServerPermission(&'a str),
    /// Sent N players to a server.
    SentMany {
        /// Number of players moved.
        count: usize,
        /// Target server name.
        server: &'a str,
    },
    /// Sent one player to a server.
    SentOne {
        /// Player username.
        player: &'a str,
        /// Target server name.
        server: &'a str,
    },
    /// Player location response (`/find`).
    PlayerOnServer {
        /// Player username.
        player: &'a str,
        /// Current server name.
        server: &'a str,
    },
    /// Kicked player confirmation.
    KickedPlayer {
        /// Player username.
        player: &'a str,
        /// Kick reason.
        reason: &'a str,
    },
    /// Config reloaded confirmation.
    Reloaded(usize),
    /// Reconnected after seamless restart.
    Reconnected,
    /// Limbo help/commands message.
    LimboHelp,
    /// Server language command hint.
    ServerLangOnly,
}

/// Translates `msg` into `lang`.
#[must_use]
pub fn translate(lang: ClientLang, msg: ClientMsg<'_>) -> String {
    match msg {
        ClientMsg::LangCurrent(cur) => match lang {
            ClientLang::Uk => format!(
                "Поточна мова: {} ({}). Змінити: /lang <код|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Ru => format!(
                "Текущий язык: {} ({}). Сменить: /lang <код|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Pl => format!(
                "Obecny język: {} ({}). Zmień: /lang <kod|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::De => format!(
                "Aktuelle Sprache: {} ({}). Ändern: /lang <code|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Fr => format!(
                "Langue actuelle : {} ({}). Changer : /lang <code|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Es => format!(
                "Idioma actual: {} ({}). Cambiar: /lang <código|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Pt => format!(
                "Idioma atual: {} ({}). Alterar: /lang <código|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::It => format!(
                "Lingua attuale: {} ({}). Cambia: /lang <codice|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Tr => format!(
                "Mevcut dil: {} ({}). Değiştir: /lang <kod|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Nl => format!(
                "Huidige taal: {} ({}). Wijzig: /lang <code|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Zh => format!(
                "当前语言：{} ({})。切换：/lang <代码|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Ja => format!(
                "現在の言語: {} ({})。変更: /lang <コード|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::Ko => format!(
                "현재 언어: {} ({}). 변경: /lang <코드|auto>",
                cur.native_name(),
                cur.code()
            ),
            ClientLang::En => format!(
                "Current language: {} ({}). Change: /lang <code|auto>",
                cur.native_name(),
                cur.code()
            ),
        },
        ClientMsg::LangSet(new_lang) => match lang {
            ClientLang::Uk => format!(
                "Мову змінено на {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Ru => format!(
                "Язык изменён на {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Pl => format!(
                "Zmieniono język na {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::De => format!(
                "Sprache geändert auf {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Fr => format!(
                "Langue définie sur {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Es => format!(
                "Idioma cambiado a {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Pt => format!(
                "Idioma alterado para {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::It => format!(
                "Lingua impostata su {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Tr => format!(
                "Dil {} ({}) olarak ayarlandı.",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Nl => format!(
                "Taal ingesteld op {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Zh => format!(
                "语言已切换为 {} ({})。",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Ja => format!(
                "言語を {} ({}) に変更しました。",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::Ko => format!(
                "언어가 {} ({})로 변경되었습니다.",
                new_lang.native_name(),
                new_lang.code()
            ),
            ClientLang::En => format!(
                "Language set to {} ({}).",
                new_lang.native_name(),
                new_lang.code()
            ),
        },
        ClientMsg::LangAuto(detected) => match lang {
            ClientLang::Uk => format!(
                "Увімкнено автовизначення мови клієнта: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Ru => format!(
                "Включено автоопределение языка клиента: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Pl => format!(
                "Włączono automatyczny język klienta: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::De => format!(
                "Automatische Client-Sprache aktiviert: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Fr => format!(
                "Langue automatique du client activée : {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Es => format!(
                "Idioma automático del cliente activado: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Pt => format!(
                "Idioma automático do cliente ativado: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::It => format!(
                "Lingua automatica del client attivata: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Tr => format!(
                "Otomatik istemci dili etkinleştirildi: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Nl => format!(
                "Automatische clienttaal ingeschakeld: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Zh => format!(
                "已启用客户端自动语言：{} ({})。",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Ja => format!(
                "クライアント言語の自動検出を有効化しました: {} ({})。",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::Ko => format!(
                "클라이언트 언어 자동 감지가 활성화되었습니다: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
            ClientLang::En => format!(
                "Auto client language enabled: {} ({}).",
                detected.native_name(),
                detected.code()
            ),
        },
        ClientMsg::LangUsage => match lang {
            ClientLang::Uk => {
                "Використання: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::Ru => {
                "Использование: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::Pl => {
                "Użycie: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::De => {
                "Verwendung: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::Fr => {
                "Utilisation : /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::Es => "Uso: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into(),
            ClientLang::Pt => "Uso: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into(),
            ClientLang::It => "Uso: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into(),
            ClientLang::Tr => {
                "Kullanım: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::Nl => {
                "Gebruik: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::Zh => "用法：/lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into(),
            ClientLang::Ja => {
                "使用法: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::Ko => {
                "사용법: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
            ClientLang::En => {
                "Usage: /lang <auto|en|uk|ru|pl|de|fr|es|pt|it|tr|nl|zh|ja|ko>".into()
            }
        },
        ClientMsg::NoPermission(cmd) => match lang {
            ClientLang::Uk => format!("У вас немає прав для використання {cmd}."),
            ClientLang::Ru => format!("У вас нет прав для использования {cmd}."),
            ClientLang::Pl => format!("Nie masz uprawnień do użycia {cmd}."),
            ClientLang::De => format!("Du hast keine Berechtigung für {cmd}."),
            ClientLang::Fr => format!("Vous n'avez pas la permission d'utiliser {cmd}."),
            ClientLang::Es => format!("No tienes permiso para usar {cmd}."),
            ClientLang::Pt => format!("Você não tem permissão para usar {cmd}."),
            ClientLang::It => format!("Non hai il permesso di usare {cmd}."),
            ClientLang::Tr => format!("{cmd} komutunu kullanma izniniz yok."),
            ClientLang::Nl => format!("Je hebt geen toestemming om {cmd} te gebruiken."),
            ClientLang::Zh => format!("你没有权限使用 {cmd}。"),
            ClientLang::Ja => format!("{cmd} を使用する権限がありません。"),
            ClientLang::Ko => format!("{cmd} 명령어를 사용할 권한이 없습니다."),
            ClientLang::En => format!("You do not have permission to use {cmd}."),
        },
        ClientMsg::PlayerNotOnline(player) => match lang {
            ClientLang::Uk => format!("Гравець '{player}' зараз не в мережі."),
            ClientLang::Ru => format!("Игрок '{player}' сейчас не в сети."),
            ClientLang::Pl => format!("Gracz '{player}' nie jest online."),
            ClientLang::De => format!("Spieler '{player}' ist nicht online."),
            ClientLang::Fr => format!("Le joueur '{player}' n'est pas en ligne."),
            ClientLang::Es => format!("El jugador '{player}' no está en línea."),
            ClientLang::Pt => format!("O jogador '{player}' não está online."),
            ClientLang::It => format!("Il giocatore '{player}' non è online."),
            ClientLang::Tr => format!("'{player}' adlı oyuncu çevrimiçi değil."),
            ClientLang::Nl => format!("Speler '{player}' is niet online."),
            ClientLang::Zh => format!("玩家 '{player}' 当前不在线。"),
            ClientLang::Ja => format!("プレイヤー '{player}' はオンラインではありません。"),
            ClientLang::Ko => format!("플레이어 '{player}'님은 현재 오프라인입니다."),
            ClientLang::En => format!("Player '{player}' is not online."),
        },
        ClientMsg::NoServerPermission(server) => match lang {
            ClientLang::Uk => format!("У вас немає доступу до сервера '{server}'."),
            ClientLang::Ru => format!("У вас нет доступа к серверу '{server}'."),
            ClientLang::Pl => format!("Nie masz dostępu do serwera '{server}'."),
            ClientLang::De => format!("Du hast keinen Zugriff auf '{server}'."),
            ClientLang::Fr => format!("Vous n'avez pas la permission de rejoindre '{server}'."),
            ClientLang::Es => format!("No tienes permiso para entrar a '{server}'."),
            ClientLang::Pt => format!("Você não tem permissão para entrar em '{server}'."),
            ClientLang::It => format!("Non hai il permesso di entrare in '{server}'."),
            ClientLang::Tr => format!("'{server}' sunucusuna katılma izniniz yok."),
            ClientLang::Nl => format!("Je hebt geen toegang tot '{server}'."),
            ClientLang::Zh => format!("你没有权限进入服务器 '{server}'。"),
            ClientLang::Ja => format!("サーバー '{server}' に参加する権限がありません。"),
            ClientLang::Ko => format!("'{server}' 서버에 접속할 권한이 없습니다."),
            ClientLang::En => format!("You do not have permission to join '{server}'."),
        },
        ClientMsg::SentMany { count, server } => match lang {
            ClientLang::Uk => format!("Відправлено {count} гравців на {server}."),
            ClientLang::Ru => format!("Отправлено {count} игроков на {server}."),
            ClientLang::Pl => format!("Wysłano {count} graczy na {server}."),
            ClientLang::De => format!("{count} Spieler nach {server} gesendet."),
            ClientLang::Fr => format!("{count} joueur(s) envoyé(s) vers {server}."),
            ClientLang::Es => format!("Enviados {count} jugador(es) a {server}."),
            ClientLang::Pt => format!("Enviados {count} jogador(es) para {server}."),
            ClientLang::It => format!("Inviati {count} giocatori su {server}."),
            ClientLang::Tr => format!("{count} oyuncu {server} sunucusuna gönderildi."),
            ClientLang::Nl => format!("{count} speler(s) naar {server} gestuurd."),
            ClientLang::Zh => format!("已将 {count} 名玩家传送到 {server}。"),
            ClientLang::Ja => format!("{count} 人のプレイヤーを {server} に転送しました。"),
            ClientLang::Ko => format!("{count}명의 플레이어를 {server}(으)로 이동했습니다."),
            ClientLang::En => format!("Sent {count} player(s) to {server}."),
        },
        ClientMsg::SentOne { player, server } => match lang {
            ClientLang::Uk => format!("Відправлено {player} на {server}."),
            ClientLang::Ru => format!("Игрок {player} отправлен на {server}."),
            ClientLang::Pl => format!("Wysłano {player} na {server}."),
            ClientLang::De => format!("{player} wurde nach {server} gesendet."),
            ClientLang::Fr => format!("{player} envoyé vers {server}."),
            ClientLang::Es => format!("{player} enviado a {server}."),
            ClientLang::Pt => format!("{player} enviado para {server}."),
            ClientLang::It => format!("{player} inviato su {server}."),
            ClientLang::Tr => format!("{player}, {server} sunucusuna gönderildi."),
            ClientLang::Nl => format!("{player} is naar {server} gestuurd."),
            ClientLang::Zh => format!("已将 {player} 传送到 {server}。"),
            ClientLang::Ja => format!("{player} を {server} に転送しました。"),
            ClientLang::Ko => format!("{player}님을 {server}(으)로 이동했습니다."),
            ClientLang::En => format!("Sent {player} to {server}."),
        },
        ClientMsg::PlayerOnServer { player, server } => match lang {
            ClientLang::Uk => format!("{player} зараз на сервері {server}."),
            ClientLang::Ru => format!("{player} сейчас на сервере {server}."),
            ClientLang::Pl => format!("{player} jest obecnie na {server}."),
            ClientLang::De => format!("{player} befindet sich auf {server}."),
            ClientLang::Fr => format!("{player} est actuellement sur {server}."),
            ClientLang::Es => format!("{player} está actualmente en {server}."),
            ClientLang::Pt => format!("{player} está atualmente em {server}."),
            ClientLang::It => format!("{player} è attualmente su {server}."),
            ClientLang::Tr => format!("{player} şu anda {server} sunucusunda."),
            ClientLang::Nl => format!("{player} bevindt zich op {server}."),
            ClientLang::Zh => format!("{player} 当前位于服务器 {server}。"),
            ClientLang::Ja => format!("{player} は現在 {server} にいます。"),
            ClientLang::Ko => format!("{player}님은 현재 {server} 서버에 있습니다."),
            ClientLang::En => format!("{player} is currently on {server}."),
        },
        ClientMsg::KickedPlayer { player, reason } => match lang {
            ClientLang::Uk => format!("Кікнуто {player}: {reason}"),
            ClientLang::Ru => format!("Кикнут {player}: {reason}"),
            ClientLang::Pl => format!("Wyrzucono {player}: {reason}"),
            ClientLang::De => format!("{player} gekickt: {reason}"),
            ClientLang::Fr => format!("{player} expulsé : {reason}"),
            ClientLang::Es => format!("{player} expulsado: {reason}"),
            ClientLang::Pt => format!("{player} expulso: {reason}"),
            ClientLang::It => format!("{player} espulso: {reason}"),
            ClientLang::Tr => format!("{player} atıldı: {reason}"),
            ClientLang::Nl => format!("{player} gekickt: {reason}"),
            ClientLang::Zh => format!("已踢出 {player}：{reason}"),
            ClientLang::Ja => format!("{player} をキックしました: {reason}"),
            ClientLang::Ko => format!("{player}님을 추방했습니다: {reason}"),
            ClientLang::En => format!("Kicked {player}: {reason}"),
        },
        ClientMsg::Reloaded(radons) => match lang {
            ClientLang::Uk => {
                format!("Конфіг проксі перезавантажено (активних радонів: {radons}).")
            }
            ClientLang::Ru => format!("Конфиг прокси перезагружен (активных радонов: {radons})."),
            ClientLang::Pl => {
                format!("Przeładowano konfigurację proxy (aktywne radony: {radons}).")
            }
            ClientLang::De => format!("Proxy-Konfiguration neu geladen ({radons} aktive Radons)."),
            ClientLang::Fr => format!("Configuration proxy rechargée ({radons} radons actifs)."),
            ClientLang::Es => {
                format!("Configuración del proxy recargada ({radons} radons activos).")
            }
            ClientLang::Pt => {
                format!("Configuração do proxy recarregada ({radons} radons ativos).")
            }
            ClientLang::It => format!("Configurazione proxy ricaricata ({radons} radon attivi)."),
            ClientLang::Tr => format!("Proxy yapılandırması yenilendi ({radons} aktif radon)."),
            ClientLang::Nl => format!("Proxy-configuratie herladen ({radons} actieve radons)."),
            ClientLang::Zh => format!("已重载代理配置（{radons} 个活跃 Radon）。"),
            ClientLang::Ja => {
                format!("プロキシ設定を再読み込みしました（有効な Radon: {radons}）。")
            }
            ClientLang::Ko => format!("프록시 설정을 다시 불러왔습니다 (활성 Radon: {radons}개)."),
            ClientLang::En => format!("Reloaded proxy config ({radons} radons active)."),
        },
        ClientMsg::Reconnected => match lang {
            ClientLang::Uk => "Успішно перепідключено до перезапущеного сервера.".into(),
            ClientLang::Ru => "Успешно переподключено к перезапущенному серверу.".into(),
            ClientLang::Pl => "Połączono ponownie ze zrestartowanym serwerem.".into(),
            ClientLang::De => "Erfolgreich mit dem neu gestarteten Server verbunden.".into(),
            ClientLang::Fr => "Reconnecté au serveur redémarré.".into(),
            ClientLang::Es => "Reconectado al servidor reiniciado.".into(),
            ClientLang::Pt => "Reconectado ao servidor reiniciado.".into(),
            ClientLang::It => "Riconnesso al server riavviato.".into(),
            ClientLang::Tr => "Yeniden başlatılan sunucuya yeniden bağlanıldı.".into(),
            ClientLang::Nl => "Opnieuw verbonden met de herstarte server.".into(),
            ClientLang::Zh => "已重新连接到重启后的服务器。".into(),
            ClientLang::Ja => "再起動したサーバーに再接続しました。".into(),
            ClientLang::Ko => "재시작된 서버에 다시 연결되었습니다.".into(),
            ClientLang::En => "Reconnected to restarted server.".into(),
        },
        ClientMsg::LimboHelp => match lang {
            ClientLang::Uk => "Команди RadLimbo: /lobby, /hub, /lang <код|auto>".into(),
            ClientLang::Ru => "Команды RadLimbo: /lobby, /hub, /lang <код|auto>".into(),
            ClientLang::Pl => "Komendy RadLimbo: /lobby, /hub, /lang <kod|auto>".into(),
            ClientLang::De => "RadLimbo-Befehle: /lobby, /hub, /lang <code|auto>".into(),
            ClientLang::Fr => "Commandes RadLimbo : /lobby, /hub, /lang <code|auto>".into(),
            ClientLang::Es => "Comandos de RadLimbo: /lobby, /hub, /lang <código|auto>".into(),
            ClientLang::Pt => "Comandos do RadLimbo: /lobby, /hub, /lang <código|auto>".into(),
            ClientLang::It => "Comandi RadLimbo: /lobby, /hub, /lang <codice|auto>".into(),
            ClientLang::Tr => "RadLimbo komutları: /lobby, /hub, /lang <kod|auto>".into(),
            ClientLang::Nl => "RadLimbo-commando's: /lobby, /hub, /lang <code|auto>".into(),
            ClientLang::Zh => "RadLimbo 命令：/lobby, /hub, /lang <代码|auto>".into(),
            ClientLang::Ja => "RadLimbo コマンド: /lobby, /hub, /lang <コード|auto>".into(),
            ClientLang::Ko => "RadLimbo 명령어: /lobby, /hub, /lang <코드|auto>".into(),
            ClientLang::En => "RadLimbo commands: /lobby, /hub, /lang <code|auto>".into(),
        },
        ClientMsg::ServerLangOnly => match lang {
            ClientLang::Uk => {
                "Мова клієнта для команд сервера оновлена. Використовуйте /lang <код|auto>.".into()
            }
            ClientLang::Ru => {
                "Язык клиента для команд сервера обновлён. Используйте /lang <код|auto>.".into()
            }
            _ => "Server command language updated. Use /lang <code|auto>.".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_minecraft_locales_and_codes() {
        assert_eq!(ClientLang::from_locale("uk_ua"), ClientLang::Uk);
        assert_eq!(ClientLang::from_locale("EN_US"), ClientLang::En);
        assert_eq!(ClientLang::from_locale("pl_pl"), ClientLang::Pl);
        assert_eq!(ClientLang::from_locale("ja_jp"), ClientLang::Ja);
        assert_eq!(ClientLang::from_locale("unknown_xx"), ClientLang::En);
        assert!(translate(ClientLang::Uk, ClientMsg::Reconnected).contains("перезапущеного"));
    }
}
