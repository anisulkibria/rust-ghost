# Earshot

A podcast theme for Vuuut. The newest episode plays from the homepage, every
episode in a list is drawn as a tape as long as the episode, and listeners
pick up where they left off.

## Publishing an episode

An episode is an ordinary post.

- **The audio.** Add an audio card. The first audio card in a post is the
  episode: the player at the top of the page plays it, and the card itself is
  hidden there. Put it near the start of the show notes, so lists and the
  homepage can find it too.
- **Chapters.** Start a line in the show notes with a timestamp, and it
  becomes a chapter: `04:12 The first shift`, `[1:02:33] Your questions`.
  Two or more rising timestamps in a row make a chapter list. Listeners can
  jump to any chapter, and the tape is split at each one.
- **Transcript.** Add a toggle card headed "Transcript". Begin each turn with
  the speaker's name in bold (`**Amaka:**`). Timestamps in brackets anywhere in
  the notes, such as `[12:04]`, become buttons that jump the player.
- **Subtitle.** The post excerpt sits under the title. It is a good place for
  the guest: "With Tom Reyes, head baker at Calder Street Bakery."
- **Episode art.** The feature image is the episode's artwork.
- **Where to start.** Feature an episode to list it under "New here? Start
  with these" on the homepage.
- **Bonus episodes.** Set a post's access to members or paid members. Its
  page and its place in lists ask listeners to sign up or upgrade.

Episodes hosted elsewhere (Transistor, Spotify, YouTube embeds) still work as
show notes; Earshot's own player appears when the post has an audio card.

## Settings

In **Design**, Earshot adds:

- **Cover art**: your square show artwork.
- **Apple Podcasts, Spotify, YouTube**: links to the show on each.
- **Podcast RSS feed**: the feed from your podcast host, for every other app.
  Listeners copy it from the Follow panel.
- **Number episodes**: numbers posts in publishing order. Turn it off if you
  publish posts that aren't episodes.
- **Signup heading and text**: the email signup box.
- Homepage and episode page sections can each be turned off.

## Templates

- **All episodes**: a page listing the 100 newest episodes as a numbered log.
- **Support the show**: a page listing your membership tiers with prices.

## Audio hosting

Seeking, skipping and resuming need the audio host to answer HTTP range
requests (`206 Partial Content`). Podcast hosts do. Check that files uploaded
to the site itself are served the same way.

## Credits

Set in [Radio Canada](https://github.com/cbcrc/radiocanadafonts), the
typeface made for the public broadcaster CBC/Radio-Canada, used under the SIL
Open Font License (`assets/fonts/OFL.txt`). The theme itself is MIT licensed.
