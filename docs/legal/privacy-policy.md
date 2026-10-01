# Privacy Policy

Last updated: 1 October 2026

This policy explains what data Canopy collects, why we collect it and what you can do about it.

## Who we are

Canopy is made by NecturaLabs, which is responsible for the data described here. If you have a question about your data, email support@necturalabs.com.

You can read this policy again at any time. In Canopy, open Settings, go to the Privacy section and choose Privacy Policy. It is also shown in the About dialog.

## The short version

- Canopy sends crash reports only if you agree. It asks the first time you start it and sends nothing until you answer.
- Your repositories, code, commit messages, branch names, file names and access tokens are never sent to us.
- Canopy checks once a day whether a new version exists by downloading a small file from GitHub. You can turn this off.
- When you use pull or merge requests, Canopy talks directly to GitHub or GitLab on your behalf. We are not part of that conversation.

## What Canopy collects

Canopy collects crash reports, and only if you agree to send them. Apart from that, it collects nothing about you.

Canopy also saves on your computer which versions of the Terms of Service and this policy you have accepted, and your answer about crash reports, so it can ask you again when one of them changes. This record stays on your computer.

## What a crash report contains

A crash report is sent when Canopy crashes, or when one of its background tasks (such as the one that reads a repository) stops because of an error in Canopy itself. Errors that Git reports, such as a rejected push, are shown to you and never sent. A crash report contains:

- the error message and the place in Canopy's code where it happened, with the chain of Canopy functions that led there (the stack trace);
- the version and build of Canopy;
- details of your system: the name and version of your operating system, the processor architecture, and the version of the Rust compiler Canopy was built with;
- a record that Canopy started and whether it closed normally or crashed.

That last record is tied to a random number that Canopy makes up each time it starts and never stores, so separate sessions cannot be linked to each other or to you. We use it only to count how often Canopy crashes.

A crash report does not contain your name, email address, Git identity, access tokens, passwords, the contents of your repositories, your commit messages or Canopy's logs. Folder names that would show your user name are hidden before a report is sent. An error message can occasionally mention a file, folder or branch name that Canopy was working with when it crashed.

Sending a report reveals your IP address to the server that receives it, as any internet connection does, but the address is not stored with the report.

Every crash is also written to a file on your computer (see "Data that stays on your computer"), whether or not you agreed to send reports.

## How we use crash reports

We use crash reports only to find and fix problems in Canopy. We do not use them for advertising, profiling or tracking, and we do not sell them.

## Legal basis

We collect crash reports only with your consent (Article 6(1)(a) of the GDPR and UK GDPR). You can withdraw your consent at any time. Withdrawing it does not affect reports that were sent before.

## Who processes crash reports

Crash reports are sent to Sentry (Functional Software, Inc.), a crash-reporting service that processes them for us under a data processing agreement. Our Sentry project stores its data in the EU, in Germany. Sentry and its subprocessors may access the data from the United States or Canada, for example to provide support. Such transfers are covered by the EU-US Data Privacy Framework and the European Commission's Standard Contractual Clauses. Sentry lists its subprocessors at https://sentry.io/legal/subprocessors/.

## How long we keep crash reports

Crash reports are deleted automatically after 90 days.

## Changing your answer

You can turn crash reports on or off at any time with the Send crash reports option in the Privacy section of Settings. The change takes effect immediately.

Sending crash reports is optional. If you choose not to send them, Canopy works the same in every way.

## Update checks

Once a day, and when you choose Check for updates, Canopy downloads a small file from GitHub (raw.githubusercontent.com) that names the latest version of Canopy. Like any web request, this tells GitHub your IP address, and the request names the version of Canopy you use. The request contains nothing else about you, and NecturaLabs does not receive it. GitHub's privacy statement covers what GitHub does with it.

Canopy never downloads or installs an update by itself. To stop update checks, turn off Check for updates in the Updates section of Settings.

## GitHub and GitLab

If you use Canopy's pull request or merge request features, Canopy sends your requests directly to GitHub or to your GitLab server, using an access token you saved in Canopy, one set in your environment (GH_TOKEN, GITHUB_TOKEN, GITLAB_TOKEN or GL_TOKEN) or one your Git credential helper already holds. Tokens from the environment are only sent to github.com, gitlab.com and GitLab servers you listed in Settings. What you see and do there (lists, comments, reviews, merges) passes only between your computer and that service, and its own privacy policy applies. NecturaLabs never receives your tokens or this data.

Tokens you save in Canopy are kept in a file in Canopy's data folder that only your user account can read. You can remove them at any time in the Accounts section of Settings, or turn these features off in the Integrations section.

## Git

Fetching, pulling, pushing and cloning run the Git program installed on your computer, which talks to the remotes you configured. That traffic goes directly between your computer and those servers.

## Data that stays on your computer

Canopy keeps its settings, your list of recent repositories, its log files and its crash reports in its data folder (for example ~/.local/share/canopy on Linux). Logs and crash reports never leave your computer automatically. If you choose to share one with us, check it first: it can contain file paths and branch names. You can open the folder from the command palette with Open log folder, and delete it at any time.

## Your rights

Depending on where you live, you may have the right to access the data we hold about you, to have it corrected or deleted, to restrict or object to its use, to receive it in a portable form and to withdraw your consent. Because crash reports contain no name or account, we may need the approximate time of the crash and your Canopy version to find yours. To use any of these rights, email support@necturalabs.com.

You can also complain to a data protection authority. In the EU, that is the authority in the country where you live or work. In the United Kingdom, it is the Information Commissioner's Office.

## Children

Apart from crash reports, which contain no name or account, Canopy collects no data from anyone, including children.

## Changes to this policy

If we change what Canopy collects, we will update this policy and its date, and Canopy will ask you again before it collects anything new.
