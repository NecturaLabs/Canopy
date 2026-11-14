# Privacy Policy

Last updated: 14 November 2026

This policy explains what Canopy sends over the internet, who receives it and what you can do about it.

## Who we are

Canopy is made by NecturaLabs, the controller of the crash data described in this policy. For questions about your data, email support@necturalabs.com.

To read this policy in Canopy, open Settings, choose Privacy and select Read next to Privacy Policy. The About dialog also has a Privacy Policy button.

## Summary

- Canopy sends crash data to NecturaLabs only if you agree. It asks the first time you start it and sends nothing before you answer.
- Canopy never sends your repositories or access tokens to NecturaLabs.
- Canopy checks GitHub for a new version at most once a day. You can turn this off.
- For repositories on GitHub or GitLab, Canopy talks to that service directly. NecturaLabs receives none of that traffic.

## What NecturaLabs receives

If you turn on crash reports, Canopy sends us crash reports, described below. If you leave them off, NecturaLabs receives nothing from Canopy.

Canopy also records, on your computer, which versions of the Terms of Service and this policy you accepted and your answer about crash reports. This record stays on your computer.

## Crash reports

Canopy sends a crash report when it, or one of its background tasks, stops because of a bug in Canopy or in a library it uses. Errors that Git reports, such as a rejected push, are shown to you and are not sent.

A crash report contains:

- the error message
- the source file and line, in Canopy or in one of its libraries, where the error happened
- the stack trace: the chain of function calls that led to the error, as memory addresses and, in development builds, function names
- the Canopy version, and whether it is a release or a development build
- the name and version of your operating system, your processor architecture and the version of the Rust compiler that built Canopy
- a random report ID, the time of the crash and the version of the crash-reporting library

Before sending a report, Canopy removes your computer's name and replaces your home folder and other user folders in file paths with ~ or <user>.

The error message comes from the code that failed. It can name a file, folder, branch or remote that Canopy was working with, and occasionally quote other text it was handling. Canopy does not add your name, email address, Git identity, access tokens, logs or repository contents to a report.

## Your IP address

Every internet connection reveals your IP address to the server at the other end, so Sentry sees it when Canopy sends crash data. Canopy does not put it in the data, and our Sentry project is set not to store it.

## How we use crash data

We use crash reports only to find and fix bugs. We do not use them for advertising or profiling, we do not combine them with other data, and we do not sell them. We make no automated decisions about you.

## Legal basis

We process crash data only with your consent (Article 6(1)(a) of the GDPR and the UK GDPR). Giving it is optional, and Canopy works the same either way. You can withdraw your consent at any time. Withdrawing it does not affect data sent before.

## Who processes crash data

Sentry (Functional Software, Inc., United States) receives and stores crash data for us as our processor, under a data processing agreement. Our Sentry project keeps its data in the EU, in Germany. Sentry and its subprocessors may access the data from the United States or Canada, for example to provide support. These transfers are covered by the EU-US Data Privacy Framework and the European Commission's Standard Contractual Clauses. Sentry lists its subprocessors at https://sentry.io/legal/subprocessors/.

## How long we keep crash data

Sentry deletes crash reports automatically 30 days after it receives them.

## Changing your answer

To turn crash reports on or off, use Send crash reports in the Privacy section of Settings. The change takes effect immediately.

## Update checks

When Canopy starts, it checks for a new version if it has not checked in the last 24 hours. It also checks when you choose Check for updates in the About dialog or the command palette, or Check now in the Updates section of Settings. The first automatic check waits until you have accepted the terms and answered about crash reports.

To check, Canopy downloads a small file that names the latest version from the NecturaLabs/app-updates repository on GitHub (raw.githubusercontent.com). If Include beta builds is on, it downloads a second file for beta builds. The request reveals your IP address to GitHub and names your Canopy build number in its User-Agent header. It contains nothing else about you. NecturaLabs does not receive the request or any record of it. GitHub's privacy statement covers what GitHub does with it.

Canopy checks for updates so that you learn about bug and security fixes. Our legal basis is this legitimate interest (Article 6(1)(f) of the GDPR). To object, turn off Check for updates in the Updates section of Settings. Canopy never downloads or installs an update by itself.

## Installing an update

Only when you choose Update in the update dialog, Canopy downloads the new build from the same NecturaLabs/app-updates repository: from github.com, which redirects to GitHub's file servers (githubusercontent.com), over HTTPS. It downloads the build's archive for your system and, to refresh the program that uninstalls Canopy, the build's checksum list and installer script. Each request reveals your IP address to GitHub and names your Canopy build number in its User-Agent header, and contains nothing else about you. NecturaLabs does not receive these requests or any record of them. GitHub's privacy statement covers what GitHub does with them.

Canopy checks the archive against the SHA-256 checksum in the update file before it installs anything, and stops if they differ. Where Canopy was put in its own folder by Canopy's installer, it replaces its program files there and leaves your settings, logs and repositories alone. Anywhere else, Update downloads Canopy's installer script from the update repository and runs it on your computer, which installs a copy of Canopy in its own folder and leaves the one you are running as it is. Canopy downloads nothing until you choose Update.

## GitHub and GitLab

When you open a repository whose remote is on GitHub (github.com) or GitLab (gitlab.com or a self-hosted GitLab server), Canopy loads its pull or merge requests from that service's API. When you comment, review, merge or create a request in Canopy, it sends that to the same API. When Canopy has a sign-in for that server, each time it loads the list of requests it also asks the same server, in one request, where each listed request's checks and reviews stand; that request names the project and the request numbers and carries your token like the others. Without a sign-in, Canopy does not send it. When a repository has more requests than the first page of the list and you type in the request filter or choose a draft, author or sort filter, Canopy sends those words and choices to the same server to search the requests that are not loaded yet. To stop all of this, turn off Pull and merge requests in the Integrations section of Settings.

To sign in to the API, Canopy uses only the account that the GitHub CLI (gh) or GitLab CLI (glab) is signed in with on your computer. Canopy runs gh or glab to read which account is signed in to that server and the token the CLI holds for it: when a repository's requests start loading, when you open the Accounts section of Settings (for github.com, gitlab.com, each GitLab server you listed and up to four servers of the repositories you have open) and when you choose Check again there. Canopy keeps the token in memory only; it never saves it, logs it or asks you to type or paste one. The CLI may contact that server itself to check the token. Canopy does not use tokens from your environment (GH_TOKEN, GITLAB_TOKEN and similar) or from your Git credential helper, and while Pull and merge requests are turned off it does not run gh or glab at all.

Without a CLI signed in to that server, Canopy reads public projects without signing in. The token is only sent to the server it belongs to.

These requests go directly between your computer and that server, which receives your IP address, your token and your Canopy version. NecturaLabs receives none of it. The service's own privacy policy applies.

Canopy saves no token. Earlier versions kept the tokens you entered in a file named tokens.json in the folder they used for Canopy's data; Canopy deletes that file when it starts. The deletion does not erase the disk blocks the file used.

## Commit author and committer profile pictures

When Download profile pictures in Settings → Privacy is on, Canopy looks up pictures for authors visible in History and the committer of the selected commit. This also requires Author avatars in Appearance and Pull and merge requests in Integrations to be on. Nothing is requested before you accept the terms and this policy and answer the crash-report question.

For GitHub, the repository's API receives a commit ID to identify its author or committer account. When GitHub answers that it does not know the commit, Canopy asks the repository's API once for the repository itself, to tell a commit that was not pushed from a repository it cannot see. That request carries no author data, and it is not sent once picture downloads are off. For GitLab, the repository's API receives the author’s or committer’s email address recorded in the commit to look up its public account picture. API requests may use your existing hosting token, sent only to its own host.

Canopy then downloads the direct image URL. It permits only HTTPS images on the repository's host, GitHub's avatars.githubusercontent.com, or, when GitLab returns such a link, Gravatar (gravatar.com, www.gravatar.com, secure.gravatar.com, 0.gravatar.com, 1.gravatar.com or 2.gravatar.com) and Libravatar (seccdn.libravatar.org). Canopy asks Gravatar and Libravatar to answer that an author has no picture (d=404 in the link) rather than send a generated one, so those authors show initials. It follows no image redirects and uses no Gravatar or Libravatar API key. Image requests carry no token, cookie or referrer. The image host receives your IP address, Canopy version and the image URL, which may include an email hash supplied by GitLab. A hash is not a guarantee of anonymity. These services' own privacy policies apply. NecturaLabs receives none of this traffic.

Pictures are stored as small thumbnails in the avatars folder inside Canopy's cache folder, with a timestamp and an opaque filename derived from the hosting server and author email. Canopy caches found pictures, and authors for whom the hosting service or image host has none, for 12 hours across restarts. When an author is shown after that interval, it tries once again. When the image host answers that an author has no picture, a picture cached earlier is removed. When the hosting service refuses the repository, does not know a commit, or cannot be reached, nothing is cached for the author. Canopy then stops asking for that repository until you restart Canopy or change the repository's remote, tries up to three of the author's other commits (often ones you have not pushed yet, whose IDs the service then receives), or waits ten minutes before asking again. When the image host limits, refuses or redirects a download, nothing is cached for the author either: Canopy keeps the previous picture, or coloured initials if none was cached, and waits until the host's reset time, or an hour, before downloading pictures from it again. Pictures are not included in logs or crash reports. Turning Download profile pictures off stops new picture requests and displays initials; existing disk cache files remain until you delete that folder or purge Canopy's data during uninstall.

## Git

Clone, fetch, pull and push run the Git program installed on your computer. Git connects directly to the remotes you configured, with your own Git settings and credentials. NecturaLabs receives none of this traffic.

Updating or adding a submodule can make Git clone or fetch from that submodule's configured URL. Clone, pull, switching branches and completing a merge or rebase can also initialize and update submodules to their recorded commits. Settings → Git → Update submodules controls these automatic updates. For pull, branch switches and merge or rebase completion, an explicit submodule.recurse setting decides instead when Follow my git config is on. Manual submodule updates still run when you choose them. These requests use your Git credentials and expose your IP address and the requested object identifiers to the submodule's server. NecturaLabs receives none of this traffic.

In a partial clone (a repository cloned with a filter such as --filter=blob:none), opening the repository, refreshing it, viewing a commit or viewing a diff can make Git contact the remote the repository was cloned from, to download the file contents and folder listings that were left out of the clone. This also happens without you pressing Fetch, and only for what the repository, commit or file you opened needs. Canopy does it on its own only for repositories inside a folder you chose to trust in Settings (Privacy, Trusted folders), because Git then follows the repository's own configuration, which can run programs and contact servers. Anywhere else Canopy downloads nothing and shows a Trust button instead. Git asks for your credentials through Canopy, as for any other Git operation, and the server receives your IP address and the identifiers of the objects it is asked for. NecturaLabs receives none of this traffic.

## Data that stays on your computer

Canopy keeps its data in a few folders. Canopy creates each folder, and in each one that is empty or holds only Canopy's own files it puts an empty file called .canopy-data, which tells the uninstaller the folder is Canopy's; a folder that already holds someone else's files is used but left as it is. On Linux and macOS Canopy lets only your user account open the folders it marks; on Windows they keep the permissions they inherit from your user profile folder. On Linux they are canopy inside XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_STATE_HOME and XDG_CACHE_HOME (~/.config, ~/.local/share, ~/.local/state and ~/.cache when you have not set them). On macOS they are ~/Library/Application Support/io.necturalabs.Canopy, ~/Library/Logs/Canopy and ~/Library/Caches/io.necturalabs.Canopy. On Windows they are %APPDATA%\NecturaLabs\Canopy and %LOCALAPPDATA%\NecturaLabs\Canopy. They contain:

- settings.json: your settings, the repositories you opened recently or left open, the GitLab servers you listed, the versions of these documents you accepted and your answer about crash reports. If Canopy cannot read it, it keeps the damaged file as settings.json.bad and starts with default settings.
- app.ron: your window's position and size, and the last 100 notifications Canopy showed you (their titles, the folder name of the repository and Git's error messages, with any user name and password in a URL removed)
- logs: one log file for each time Canopy runs. A log records what Canopy did and when, and can contain file paths, branch names, server names, the Git commands Canopy ran (without passwords, tokens or commit messages) and Git's error messages. The ten newest are kept, and each stops growing at 20 MB. Log detail in the Privacy section of Settings controls how much is recorded.
- crashes: a text file for each crash, written whether or not you send crash reports. The ten newest are kept.
- avatars: the profile pictures Canopy downloaded, kept for 12 hours.

Earlier versions kept these files in other folders; Canopy moves them to the folders above the first time it starts.

These files never leave your computer unless you send them yourself. Before you send us a log or crash file, read it: it can contain file paths and branch names. To open the logs folder, choose Open log folder in the command palette or Open folder in the Privacy section of Settings. Copy diagnostics in the same section puts this on the clipboard, so you can paste it into a bug report, and sends nothing: your Canopy, Git and operating system versions, the run's id, the full path of the current log file, your log detail, theme, contrast and translucency settings, the number of repositories open, your answer about crash reports and the latest 50 warnings and errors, with your home folder shown as ~ and tokens, passwords and email addresses removed. You can delete these folders at any time. Canopy then starts as if newly installed and asks the start-up questions again.

## Your rights

Under the GDPR and the UK GDPR, you have the right to access the personal data we hold about you, to have it corrected or erased, to restrict or object to its processing, to receive it in a portable form and to withdraw your consent.

Crash data contains no name or account, so we usually cannot tell which reports are yours. If you send us the time of a crash, your Canopy version and your operating system, or the crash file from your computer, we will look for matching reports. To use any of your rights, email support@necturalabs.com.

You can also complain to a data protection authority, in particular the one in the EU country where you live or work. In the United Kingdom, that is the Information Commissioner's Office.

## Children

Canopy is a tool for software developers and is not aimed at children. If you are under 16, ask a parent or guardian before you turn on crash reports.

## Changes to this policy

If we change this policy, we will update the date above. Canopy shows you the new version the next time you start it and asks you to accept it before you continue. If a change adds to what crash data contains, Canopy will also ask you again whether to send crash reports.
