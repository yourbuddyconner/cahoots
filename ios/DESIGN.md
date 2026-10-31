# The look of the iOS app

The app should feel native, quiet and cared for, like the site: paper, ink, one blue, hairlines,
and serif titles. The quality comes from type, spacing and a few custom controls, not from
decoration. The faces are avatars. They sit where an avatar naturally goes and nowhere else.

## Tokens

Colours live in `Assets.xcassets`, each with a light and a dark value, and in `Theme.swift`.

| Token | Light | Dark | Use |
|---|---|---|---|
| Paper | #FFFFFF | #17181B | every screen's ground |
| Ink | #17181B | #F4F4F6 | text, outlines of controls |
| Blue | #304ED1 | #7B8FFF | the one accent: primary buttons, my bubbles, unread counts, links |
| Line | #E1E3EA | #2E3036 | hairlines between rows, borders of fields and cards |
| Muted | #50545E | #A2A6B3 | secondary text |
| Cream | #F0DCBD | #F0DCBD | only behind a face, as on the badge's e-paper screen |

No other colours. No gradients, no shadows, no card inside a card. Problems read in ink beside a
small mark, not in red. The system's destructive role stays red in confirmation dialogs, where iOS
draws it.

Spacing is the 4 pt grid: 8 inside a group, 16 for margins, 24 between groups, 32 to 48 around a
screen's title. Corners are 12 pt on fields, buttons and the code. Faces have square corners.

## Type

| Role | Style |
|---|---|
| Screen title | `.largeTitle`, system serif (New York), regular weight |
| Headline in a screen | `.title2`, serif |
| Circle name in a row, the six words, hello words | `.title3` to `.title`, serif |
| Body | `.body`, SF |
| Meta | `.subheadline`, Muted |
| Section label | `.footnote` semibold, Muted |
| Codes, fingerprints | `.footnote`, SF Mono |

Every size is a text style, so Dynamic Type scales all of it. Rows stack vertically at
accessibility sizes.

## Components

- Filled button: Blue with paper text, full width, 52 pt tall. One per screen.
- Outlined button: ink text in a hairline outline, for the second action.
- Text button: blue text, for a quiet third action.
- Option row: a title, one line under it, a blue check when chosen. Replaces segmented pickers
  where the choice needs words.
- Section: a small muted label above rows split by hairlines. No grey grouped background.
- Step bar: four short segments under the ceremony's title, blue up to the current step.
- Face: the 24 x 24 cahoot drawn with `Canvas`, ink on cream, square corners, a one-pixel cream
  margin. 20 pt in a circle row, 24 pt in chat, 44 pt in the roster, 96 pt in Settings. Labelled
  "Seat N's face" for VoiceOver until traits give it words.
- Liquid Glass, on iOS 26 and later only: the chat composer and the bottom action bars float on
  glass. On iOS 17 to 18 the same bars sit on Paper with a hairline above. The design does not
  depend on glass.

## Screens

- Circles: serif title "Cahoots", then each circle as a row: its name in serif, one sentence of
  meta ("Three people. You are seat 2."), the members' faces small beside it, and the unread count
  in a blue capsule. With no circles, an empty state in words: the line "Meet in person. Stay in
  cahoots." and three numbered steps, like the site. Create (filled) and Join (outlined) sit in
  the bottom bar.
- Create a circle: a name field, a people stepper with large targets, the two ways to hold the
  ceremony as option rows, the secret size as option rows. "Start the ceremony" in the bottom bar.
- Join: the camera in a framed square, the paste field under it. Then "Which seat are you?" with
  the free seats as option rows and "Make my contribution".
- Ceremony: the circle's name inline, then the step bar, the step's sentence as a serif headline,
  and the code on a paper card with a hairline. The host sees every seat as a row with its state
  in words: waiting, here but not scanned, scanned. "Everyone is scanned" then Continue.
- Words: the six words large in serif, two columns of three, numbered in blue. "They match"
  filled, "They differ" as a text button.
- Chat: the circle's name and the members' small faces in the header. My messages on the right in
  Blue; theirs on the left on Paper with a hairline, the sender's face beside the first of a run
  and their name above it. A thin divider marks new keys. The composer is one rounded field with a
  blue send arrow.
- People: the circle's faces and name introduce the roster. Each seat has a 44 pt face, a familiar
  name, its seat number, and a separate pencil button. Other members' rows open hello words.
  Hardware claims live in an expandable section so the everyday actions remain easy to find.
- Names: edit any seat, including your own. Save trims surrounding whitespace; a blank name
  restores the seat number. Names appear in chat and hello words, stay on this phone, and live
  in `seat-names.json` inside the protected circle directory. They leave when the circle does.
- Hello words: first open each other's seat and match hello numbers. The two serif word pairs
  clearly distinguish what to say from what to listen for. Never read out the expected reply.
  Catch-up guidance is expandable. Advancing requires confirmation, is guarded while saving,
  and explains that each person advances their own phone. Words wrap at larger text sizes.
- Create: name, people, and connection method are the main decisions. Shared secret size is
  under Advanced options. The primary action becomes available after entering a name.
- Settings: the phone's own face on cream at the top, the key-store sentence and fingerprint under
  it, then quiet sections: relay for new circles, checking for messages, circles.

## Copy

Plain words, sentence case, active verbs. Every button says what it does. No middle dots, no em
dashes, no semicolons, no all caps. Meta is written as sentences.

## Cut

- The coloured dot ring that stood in for a circle's picture.
- Segmented pickers where the choice needed a sentence.
- Middle-dot meta strings, the epoch on every message, "you · 1" seat chips.
- Grey grouped backgrounds and the default `Form` look.
- Red status text.

## Notes from building it

- `Theme.swift` holds the tokens, the button styles, `PaperSection`, `OptionRow`, `ScreenHeader`
  and `actionBar`. `FaceView.swift` draws a face from the 576 pixel bytes of
  `Phone.facePixels(seed:)`; the seeds come from `Phone.seatFaceSeed(circle:seat:)` and
  `Phone.myFaceSeed()`, all three on the phone queue. So `FaceView` takes pixels, not a seed.
- Large titles are the system serif through `UINavigationBar.appearance()`: only the title font
  and colour change, so the bars keep their system material.
- An action bar is attached only when it holds something. On iOS 17 an empty bar is still a
  strip of paper above the keyboard, and it took the tap meant for the paste field's Use button.
- The paste field focuses on a tap anywhere inside its border, not only on the text.
- The ceremony's step bar counts Meet the group, Exchange secrets, Confirm members, Compare words.
  A joiner still showing its code to the host is on step 1, whatever the relay's phase.
- Relay errors: no answer at all reads "Cannot reach the relay. Trying again." An answer with an
  error reads the relay's `detail` ("The relay refused the message: unknown message version"),
  or its status ("The relay answered 500."). Swift writes these words in `UrlSessionHttp`, and
  the phone crate passes them through unchanged. Telling the two apart rests on the message
  text, since `PhoneError` has one `Network` case for both.
- Screenshots were taken on the iOS 17.5 simulator, the only runtime installed, in light mode,
  dark mode, and at accessibility extra large text. The iOS 26 glass paths (`safeAreaBar`, the
  composer's `glassEffect`) compile but have not been seen running.
