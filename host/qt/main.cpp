#include <QApplication>
#include <QCommandLineParser>
#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QVersionNumber>
#include <QSysInfo>
#include <algorithm>
#include <QHostAddress>
#include <QNetworkCookie>
#include <QTcpServer>
#include <QTimer>
#include <QUrl>
#include <QWebEngineCookieStore>
#include <QWebEnginePage>
#include <QWebEngineProfile>
#include <QWebEngineScript>
#include <QWebEngineScriptCollection>
#include <QWebEngineSettings>
#include <QWebEngineUrlRequestInfo>
#include <QWebEngineUrlRequestInterceptor>
#include <QWebEngineView>
#include <csignal>
#include <cstdio>
#if defined(_WIN32)
#include <windows.h>
#elif defined(__linux__)
#include <malloc.h>
#include <sys/prctl.h>
#include <unistd.h>
#else
#include <unistd.h>
#endif

static volatile std::sig_atomic_t interrupted = 0;
static bool g_verbose = false;
static void onInterrupt(int) { interrupted = 1; }

static bool loginRequested(int argc, char **argv) {
    for (int i = 1; i < argc; ++i)
        if (qstrcmp(argv[i], "--login") == 0) return true;
    return false;
}

// Hidden playback host: nothing is rendered, so visual resources and third-party
// analytics are pure overhead. Spotify's own API/DRM traffic is never touched.
class SpotifyFilter final : public QWebEngineUrlRequestInterceptor {
public:
    SpotifyFilter(QObject *parent, bool cuts) : QWebEngineUrlRequestInterceptor(parent), m_cuts(cuts) {}

    void interceptRequest(QWebEngineUrlRequestInfo &request) override {
        using Info = QWebEngineUrlRequestInfo;
        const QString host = request.requestUrl().host();
        const QString path = request.requestUrl().path();
        const bool spotify = host == "spotify.com" || host.endsWith(".spotify.com")
            || host == "scdn.co" || host.endsWith(".scdn.co")
            || host == "spotifycdn.com" || host.endsWith(".spotifycdn.com");
        const auto type = request.resourceType();

        if (m_cuts && spotify && (type == Info::ResourceTypeImage
                || type == Info::ResourceTypeFavicon
                || type == Info::ResourceTypeFontResource)) {
            request.block(true);
            return;
        }
        // reCAPTCHA never completes on a headless signed-in session.
        const bool recaptcha = (host == "www.google.com" || host == "google.com"
            || host == "recaptcha.net" || host == "www.gstatic.com")
            && path.startsWith("/recaptcha");
        if (m_cuts && recaptcha) {
            request.block(true);
            return;
        }
        if (m_cuts && !spotify && (type == Info::ResourceTypeScript
                || type == Info::ResourceTypeSubFrame
                || type == Info::ResourceTypeWorker
                || type == Info::ResourceTypeSharedWorker)) {
            request.block(true);
        }
    }

private:
    bool m_cuts;
};

class HostPage final : public QWebEnginePage {
public:
    using QWebEnginePage::QWebEnginePage;
protected:
    void javaScriptConsoleMessage(JavaScriptConsoleMessageLevel level, const QString &message,
            int line, const QString &source) override {
        // Page console is Spotify's own noise; keep it for field debugging.
        if (g_verbose) qInfo().noquote() << "console" << (int)level << source << line << message;
    }
};

static QString dataDirectory() {
    if (qEnvironmentVariableIsSet("SPOTCORE_DATA_DIR")) {
        const QString root = qEnvironmentVariable("SPOTCORE_DATA_DIR");
        return QDir::isAbsolutePath(root) ? root : QString();
    }
#if defined(_WIN32)
    QString base = qEnvironmentVariable("LOCALAPPDATA");
    if (base.isEmpty()) base = qEnvironmentVariable("APPDATA");
#elif defined(__APPLE__)
    const QString home = qEnvironmentVariable("HOME");
    QString base = home.isEmpty() ? QString() : home + "/Library/Application Support";
#else
    QString base;
    if (qEnvironmentVariableIsSet("XDG_DATA_HOME")) base = qEnvironmentVariable("XDG_DATA_HOME");
    else if (!qEnvironmentVariableIsEmpty("HOME")) base = qEnvironmentVariable("HOME") + "/.local/share";
#endif
    if (!QDir::isAbsolutePath(base)) return QString();
    return base + "/spotcore";
}

// Reserves an ephemeral loopback port. The listener is closed before Chromium
// binds it, so another process can still take it; the launcher retries.
static quint16 reservePort() {
    QTcpServer probe;
    if (!probe.listen(QHostAddress::LocalHost, 0)) return 0;
    const quint16 port = probe.serverPort();
    probe.close();
    return port;
}

// Must match spotcore_web::host::default_profile_dir(), so a manual run and a
// managed run share one signed-in session.
static QString defaultProfileDirectory() {
    const QString root = dataDirectory();
    return root.isEmpty() ? QString() : root + "/profile";
}

static QString widevineLibraryName() {
#if defined(_WIN32)
    return "widevinecdm.dll";
#elif defined(__APPLE__)
    return "libwidevinecdm.dylib";
#else
    return "libwidevinecdm.so";
#endif
}

static QString scanWidevineTree(const QString &profileRoot) {
    const QString lib = widevineLibraryName();
    QDir versions(profileRoot + "/WidevineCdm");
    if (!versions.exists()) return QString();
    auto versionDirs = versions.entryInfoList(QDir::Dirs | QDir::NoDotAndDotDot);
    std::sort(versionDirs.begin(), versionDirs.end(), [](const QFileInfo &a, const QFileInfo &b) {
        return QVersionNumber::compare(QVersionNumber::fromString(a.fileName()), QVersionNumber::fromString(b.fileName())) > 0;
    });
#if defined(_WIN32)
    const QString platform = "win_";
#elif defined(__APPLE__)
    const QString platform = "mac_";
#else
    const QString platform = "linux_";
#endif
    QString arch = QSysInfo::currentCpuArchitecture();
    if (arch == "x86_64") arch = "x64";
    else if (arch == "i386") arch = "x86";
    for (const QFileInfo &version : versionDirs) {
        const QString candidate = version.absoluteFilePath() + "/_platform_specific/" + platform + arch + "/" + lib;
        if (QFileInfo(candidate).isFile()) return candidate;
        const QString flat = version.absoluteFilePath() + "/" + lib;
        if (QFileInfo(flat).isFile()) return flat;
    }
    for (const QString &candidate : {profileRoot + "/WidevineCdm/_platform_specific/" + platform + arch + "/" + lib,
            profileRoot + "/WidevineCdm/" + lib})
        if (QFileInfo(candidate).isFile()) return candidate;
    return QString();
}

static QStringList browserDataRoots() {
    QStringList roots;
#if defined(_WIN32)
    const QString local = qEnvironmentVariable("LOCALAPPDATA");
    if (!local.isEmpty()) {
        for (const char *sub : {"Google/Chrome/User Data", "Chromium/User Data",
                 "BraveSoftware/Brave-Browser/User Data", "Microsoft/Edge/User Data"})
            roots << local + "/" + sub;
    }
#elif defined(__APPLE__)
    const QString support = QDir::homePath() + "/Library/Application Support";
    for (const char *sub : {"Google/Chrome", "Chromium",
             "BraveSoftware/Brave-Browser", "Microsoft Edge"})
        roots << support + "/" + sub;
#else
    QString config = qEnvironmentVariable("XDG_CONFIG_HOME");
    if (!QDir::isAbsolutePath(config)) config = QDir::homePath() + "/.config";
    for (const char *sub : {"google-chrome", "chromium",
             "BraveSoftware/Brave-Browser", "microsoft-edge", "opera", "vivaldi"})
        roots << config + "/" + sub;
    roots << QDir::homePath() + "/.var/app/com.google.Chrome/config/google-chrome";
    roots << QDir::homePath() + "/.var/app/org.chromium.Chromium/config/chromium";
#endif
    return roots;
}

static QStringList widevineAppRoots() {
    QStringList roots;
#if defined(__APPLE__)
    for (const char *app : {
             "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/Current/Libraries",
             "/Applications/Brave Browser.app/Contents/Frameworks/Brave Browser Framework.framework/Versions/Current/Libraries",
             "/Applications/Microsoft Edge.app/Contents/Frameworks/Microsoft Edge Framework.framework/Versions/Current/Libraries"})
        roots << app;
#elif !defined(_WIN32)
    for (const char *dir : {"/opt/google/chrome", "/opt/brave.com/brave", "/opt/microsoft/msedge"})
        roots << dir;
#endif
    return roots;
}

static QString findWidevineLibrary() {
    const QString env = qEnvironmentVariable("SPOTCORE_WIDEVINE");
    if (!env.isEmpty() && QFileInfo(env).isFile()) return QFileInfo(env).absoluteFilePath();

    const QString local = dataDirectory() + "/" + widevineLibraryName();
    if (QFileInfo(local).isFile()) return QFileInfo(local).absoluteFilePath();

    for (const QString &root : browserDataRoots()) {
        const QString found = scanWidevineTree(root);
        if (!found.isEmpty()) return found;
    }
    for (const QString &root : widevineAppRoots()) {
        const QString found = scanWidevineTree(root);
        if (!found.isEmpty()) return found;
    }

#if !defined(_WIN32)
    for (const char *path : {"/usr/lib/chromium/libwidevinecdm.so",
             "/usr/lib64/chromium/libwidevinecdm.so",
             "/usr/lib/chromium-browser/libwidevinecdm.so",
             "/usr/lib64/chromium-browser/libwidevinecdm.so"})
        if (QFileInfo(QString::fromLatin1(path)).isFile())
            return QFileInfo(QString::fromLatin1(path)).absoluteFilePath();
#endif

    return QString();
}

int main(int argc, char **argv) {
    const bool login = loginRequested(argc, argv);
    if (login) {
        if (qEnvironmentVariable("QT_QPA_PLATFORM") == "offscreen") qunsetenv("QT_QPA_PLATFORM");
        if (qEnvironmentVariableIsEmpty("DISPLAY") && qEnvironmentVariableIsEmpty("WAYLAND_DISPLAY")) {
            std::fputs("Sign-in needs a display: DISPLAY and WAYLAND_DISPLAY are both unset.\n",
                stderr);
            return 1;
        }
    } else if (qEnvironmentVariableIsEmpty("QT_QPA_PLATFORM")) {
        qputenv("QT_QPA_PLATFORM", "offscreen");
    }
    if (qEnvironmentVariableIsEmpty("QT_OPENGL")) qputenv("QT_OPENGL", "software");
    if (qEnvironmentVariableIsEmpty("QT_QPA_OFFSCREEN_NO_GLX")) qputenv("QT_QPA_OFFSCREEN_NO_GLX", "1");

#if defined(__linux__)
    prctl(PR_SET_NAME, "spotcore-engine", 0, 0, 0);
#endif
    QApplication app(argc, argv);
    app.setApplicationName("spotcore-engine");
    QCommandLineParser parser;
    parser.setApplicationDescription(
        "SpotCore playback engine: a signed-in, hidden Spotify page controlled over CDP.");
    parser.addHelpOption();
    parser.addOption({"multi-process", "Allow helper processes when single-process playback is unavailable"});
    parser.addOption({"profile-dir", "Signed-in Qt Spotify profile directory (default: <user data>/spotcore/profile)", "directory"});
    parser.addOption({"widevine", "Installed Widevine library path (auto-detected if omitted)", "library"});
    parser.addOption({"cdp-port", "CDP port bound on 127.0.0.1 (default: an ephemeral port)", "port"});
    parser.addOption({"verbose", "Forward the page console to stderr"});
    parser.addOption({"no-cuts", "Disable resource blocking (field debugging only)"});
    parser.addOption({"login", "Show a visible Spotify page to sign in, then exit"});
    parser.addOption({"parent-pid", "Exit when this parent process disappears", "pid"});
    parser.addOption({"services-only", "Use Spotify services without requiring a local audio decoder"});
    parser.process(app);

    const QString profilePath = parser.isSet("profile-dir")
        ? QFileInfo(parser.value("profile-dir")).absoluteFilePath()
        : defaultProfileDirectory();

    if (profilePath.isEmpty() || (parser.isSet("profile-dir") && parser.value("profile-dir").isEmpty())) {
        qCritical("No persistent data directory; set SPOTCORE_DATA_DIR or --profile-dir.");
        return 1;
    }
    QString widevine;
    if (!login && !parser.isSet("services-only")) {
        if (parser.isSet("widevine")) {
            widevine = QFileInfo(parser.value("widevine")).absoluteFilePath();
        } else {
            widevine = findWidevineLibrary();
        }
        if (!widevine.isEmpty() && (!QFileInfo(widevine).isFile() || widevine.contains('\"'))) {
            qCritical("No Widevine library found. Specify --widevine /path/to/libwidevinecdm.so or set SPOTCORE_WIDEVINE.");
            return 1;
        }
    }

    g_verbose = parser.isSet("verbose") || login;

    if (!login) {
        quint16 cdpPort = parser.value("cdp-port").toUShort();
        if (cdpPort == 0) cdpPort = reservePort();
        if (cdpPort == 0) {
            qCritical("Could not reserve a loopback port for CDP.");
            return 1;
        }
        qputenv("QTWEBENGINE_REMOTE_DEBUGGING", "127.0.0.1:" + QByteArray::number(cdpPort));
        // Use an ephemeral loopback endpoint and print it for the launcher.
        std::printf("CDP endpoint: http://127.0.0.1:%u\n", unsigned(cdpPort));
        std::fflush(stdout);
    }

    // Sign-in is a one-off, visible session: no cuts, no memory tuning, and
    // Chromium's normal multi-process startup because the form and captcha need
    // it. Playback always takes the lowest-memory path.
    const bool cuts = !login && !parser.isSet("no-cuts");
    // Single-process disables sandbox isolation and is not a supported Chromium mode.
    // Keep it Linux-only; other platforms retain normal process isolation.
    QString flags = "--disable-gpu --enable-low-end-device-mode "
        "--disable-speech-api --disable-media-session-api --disable-notifications "
        "--disable-background-networking --disable-breakpad --disable-component-update "
        "--disable-domain-reliability --disable-sync --no-default-browser-check --no-pings "
        "--disable-features=Translate,OptimizationHints,MediaRouter,AudioServiceOutOfProcess "
        // 36 MiB old space is the floor: 24 MiB makes the Web Player thrash and
        // never reach playback, measured. Do not lower without a playback test.
        "--js-flags=\"--max-old-space-size=" + QString(login ? "128" : "36")
        + (login ? " --max-semi-space-size=8\"" : " --max-semi-space-size=4 --optimize-for-size\"");
    if (!widevine.isEmpty()) flags += " --widevine-path=\"" + widevine + "\"";
#if defined(__linux__)
    if (!login && !parser.isSet("multi-process")) flags += " --single-process --no-sandbox";
#endif
    qputenv("QTWEBENGINE_CHROMIUM_FLAGS", flags.toUtf8());

    QWebEngineProfile profile("spotcore-engine");
    profile.setPersistentStoragePath(profilePath);
    profile.setCachePath(QDir(QFileInfo(profilePath).absolutePath()).filePath("cache"));
    profile.setPersistentCookiesPolicy(QWebEngineProfile::ForcePersistentCookies);
    SpotifyFilter filter(&profile, cuts);
    profile.setUrlRequestInterceptor(&filter);

    if (cuts) {
        QWebEngineScript workerGuard;
        workerGuard.setName("spotcore-worker-guard");
        workerGuard.setInjectionPoint(QWebEngineScript::DocumentCreation);
        workerGuard.setWorldId(QWebEngineScript::MainWorld);
        workerGuard.setRunsOnSubFrames(true);
        workerGuard.setSourceCode(QStringLiteral(R"JS(
{
  for (const kind of ['Worker','SharedWorker']) {
    const Native = window[kind];
    if (typeof Native !== 'function') continue;
    window[kind] = new Proxy(Native, { construct(target, args, newTarget) {
      if (/^(https?|file):$/.test(location.protocol)
          && /(^|\.)google\.com$/.test(location.hostname)
          && location.pathname.startsWith('/recaptcha/'))
        throw new DOMException('reCAPTCHA worker blocked by the SpotCore host', 'NotSupportedError');
      return Reflect.construct(target, args, newTarget);
    }});
  }
}
)JS"));

        QWebEngineScript visual;
        visual.setName("spotcore-visual-off");
        visual.setInjectionPoint(QWebEngineScript::DocumentReady);
        visual.setWorldId(QWebEngineScript::MainWorld);
        visual.setRunsOnSubFrames(false);
        visual.setSourceCode(QStringLiteral(
            "if(location.origin==='https://open.spotify.com'){"
            "const style=document.createElement('style');"
            "style.textContent='*,*::before,*::after{animation:none!important;"
            "transition:none!important;scroll-behavior:auto!important}"
            " #main{display:none!important}';"
            "document.head.appendChild(style);}"));

        profile.scripts()->insert(workerGuard);
        profile.scripts()->insert(visual);
    }

    HostPage page(&profile);
    page.setVisible(true);

    QWebEngineView window;
    if (login) {
        window.setWindowTitle("Spotify");
        QObject::connect(&page, &QWebEnginePage::titleChanged, &window,
            &QWidget::setWindowTitle);
        window.resize(1024, 768);
        window.setPage(&page);
        window.show();
        QObject::connect(profile.cookieStore(), &QWebEngineCookieStore::cookieAdded, &page,
            [&app](const QNetworkCookie &cookie) {
                if (cookie.name() == "sp_dc" && cookie.domain().contains("spotify.com")) {
                    qInfo("Signed in; closing the sign-in window.");
                    QTimer::singleShot(1500, &app, &QCoreApplication::quit);
                }
            });
        profile.cookieStore()->loadAllCookies();
    }

    if (cuts) {
        auto *settings = page.settings();
        for (auto attribute : {QWebEngineSettings::AutoLoadImages,
                QWebEngineSettings::AutoLoadIconsForPage, QWebEngineSettings::TouchIconsEnabled,
                QWebEngineSettings::PdfViewerEnabled, QWebEngineSettings::WebGLEnabled,
                QWebEngineSettings::Accelerated2dCanvasEnabled,
                QWebEngineSettings::FullScreenSupportEnabled,
                QWebEngineSettings::ScreenCaptureEnabled,
                QWebEngineSettings::JavascriptCanOpenWindows,
                QWebEngineSettings::NavigateOnDropEnabled, QWebEngineSettings::ShowScrollBars,
                QWebEngineSettings::ScrollAnimatorEnabled,
                QWebEngineSettings::HyperlinkAuditingEnabled, QWebEngineSettings::PluginsEnabled})
            settings->setAttribute(attribute, false);
        settings->setImageAnimationPolicy(QWebEngineSettings::ImageAnimationPolicy::Disallow);
        profile.setSpellCheckEnabled(false);
    }

    QObject::connect(&page, &QWebEnginePage::renderProcessTerminated, &app,
        [&app](QWebEnginePage::RenderProcessTerminationStatus, int) {
            qCritical("Spotify renderer exited; stopping the host so the app can restart it.");
            app.exit(2);
        });

    std::signal(SIGINT, onInterrupt);
    std::signal(SIGTERM, onInterrupt);
    QTimer timer;
#if defined(__linux__)
    int trimCounter = 0;
#endif
    QObject::connect(&timer, &QTimer::timeout, &app, [&app
#if defined(__linux__)
        , &trimCounter
#endif
    ] {
        if (interrupted) app.quit();
#if defined(__linux__)
        if (++trimCounter % 40 == 0) {
            malloc_trim(0);
        }
#endif
    });
    timer.start(250);

    // The launcher passes its own pid: a killed app must not leave a signed-in
    // session holding the profile and the CDP port.
    if (!login && parser.isSet("parent-pid")) {
        const qint64 parentPid = parser.value("parent-pid").toLongLong();
#if defined(_WIN32)
        HANDLE parent = OpenProcess(SYNCHRONIZE, FALSE, static_cast<DWORD>(parentPid));
        auto *guard = new QTimer(&app);
        QObject::connect(guard, &QTimer::timeout, &app, [&app, parent] {
            if (!parent || WaitForSingleObject(parent, 0) != WAIT_TIMEOUT) app.quit();
        });
#else
        auto *guard = new QTimer(&app);
        QObject::connect(guard, &QTimer::timeout, &app, [&app, parentPid] {
            if (static_cast<qint64>(getppid()) != parentPid) app.quit();
        });
#endif
        guard->start(1000);
    }

    // An app that runs its own sign-in hands the session cookie over in the
    // environment, which keeps it out of argv and the process list. Writing it
    // before the first load means the page boots already signed in, exactly as
    // if --login had run once for this profile.
    if (!login) {
        const QByteArray seeded = qgetenv("SPOTCORE_SP_DC").trimmed();
        if (!seeded.isEmpty()) {
            qunsetenv("SPOTCORE_SP_DC");
            QNetworkCookie cookie("sp_dc", seeded);
            cookie.setDomain(QStringLiteral(".spotify.com"));
            cookie.setPath(QStringLiteral("/"));
            cookie.setSecure(true);
            cookie.setHttpOnly(true);
            cookie.setExpirationDate(QDateTime::currentDateTimeUtc().addYears(1));
            profile.cookieStore()->setCookie(cookie, QUrl(QStringLiteral("https://open.spotify.com/")));
        }
    }

    const bool deferred = !login && qEnvironmentVariableIsSet("SPOTCORE_DEFER_PAGE");
    qunsetenv("SPOTCORE_DEFER_PAGE");
    page.load(QUrl(deferred ? "about:blank" : login
        ? "https://accounts.spotify.com/en/login?continue=https%3A%2F%2Fopen.spotify.com%2F"
        : "https://open.spotify.com/preferences"));
    return app.exec();
}
