# Privacy policy

save-relay is an open-source program that copies game saves between your devices through storage you choose. This page covers the save-relay command-line program, its Steam Game Mode panel (Decky plugin) and its local `save-relay panel` page.

## What save-relay collects

Nothing. save-relay has no server, no account of its own, no analytics and no crash reporting. The project never receives your saves, your sign-in tokens or anything about you or your devices.

## Where your data goes

Your saves, their history and save-relay's sync records (such as which of your devices last synced a game, by the device name, and when) go only to the storage service you pick at setup, under your own account:

- **Nextcloud or another WebDAV server:** the folder you name on the server you name.
- **Google Drive:** save-relay asks only for the `drive.file` permission, so it can see and change only the files it created. It cannot read the rest of your Drive.
- **Dropbox:** save-relay uses an App folder (`Apps/save-relay/`), so it can see and change only that folder. It cannot read the rest of your Dropbox.

save-relay talks directly from your device to that service. Google's and Dropbox's own privacy policies cover what they hold.

## Other requests save-relay makes

Two other requests leave your device, and neither carries saves, tokens or anything about your accounts:

- **Steam's public store** (`store.steampowered.com`): to tell whether an installed Steam game uses Steam Cloud, save-relay asks the store about that game by its app id, one game per request. Each answer is kept on your device for a week, unless you ask save-relay to check again.
- **The ludusavi data** (`raw.githubusercontent.com`): save-relay downloads the public ludusavi manifest, which says where games keep their saves, at most about once a week. GitHub sees that request.

## Sign-in and credentials

Signing in to Google Drive or Dropbox happens in your browser (or with a code you type on your phone, in Steam's Game Mode), on the service's own page. save-relay keeps the resulting tokens, and any Nextcloud app password you give it, in your operating system's credential store on that device. They are never sent anywhere except to the service they belong to.

To revoke access, remove save-relay from your Google account's third-party access page or from Dropbox's connected apps. To switch services, run `save-relay setup` again, which also removes the old sign-in from your device.

## Local files

On your device, save-relay reads and writes the saves folders of the games you add, and keeps its configuration and sync records in its own folders. The local panel listens only on the loopback address (`127.0.0.1`) and needs a token; nothing outside your device can reach it.

## Changes and contact

Changes to this policy are made in this file in the public repository, so its history shows every version. Questions go to the repository's issues: <https://github.com/iannitram/save-relay/issues>.
