# Ceremony guide

How to run a ceremony with a group, using the app on phones. One person hosts. It takes about five minutes for four people.

![The ceremony in five steps](diagrams/ceremony.svg)

## What you need

- Two or more phones or laptops in the same room, each with a camera. Open https://cahoots.network/app on each one.
- Enough light to scan a screen. Turn screen brightness up.
- Nobody else looking at the screens. The codes you show each other are not secret, but the room is the point.

## Before you start

The host opens **Start a ceremony** and fills in three things:

1. **Name.** Anything the group will recognise, such as the name of the meetup.
2. **People in the room.** Count every device that will take part, including the host's.
3. **Secret size.** Small is enough for a first try and is the quickest to scan. Larger sizes hold more messages but take longer at step 2.

Leave **Advanced** alone unless you know you want it. It holds one option, a hosted software member. That means a program running in a secure cloud service joins as one of the members and gets its own copy of the secret. Use it only if the group has decided to work with such a program. Everyone's screen shows that it is on before they confirm.

If someone wants to add randomness from a YubiKey, they do that on their own device at the next step, from **Add hardware randomness**. It is optional.

## During

**Seats.** The host taps **Continue**, keeps seat 1, and taps **Make my contribution**. The host's screen now shows a code. Everyone else opens **Join one**, scans the host's code, and picks a seat that nobody else has. Say your seat number out loud so two people do not take the same one. Then tap **Make my contribution**.

**Step 1, Commit.** Every screen shows a code and a line saying whose codes are still needed. Show your code to each of those people, and scan theirs. The order does not matter. Each scan gives a short buzz and a green note. When your screen has everyone, it moves on by itself.

**Step 2, Reveal.** Every screen now shows a looping animation instead of a single code. Scan each person's animation. Hold both phones still until the bar fills, then move to the next person. This is the slow step. When your screen has everyone, it moves on by itself.

**Step 3, Check.** Every screen shows six words. One person reads them out loud. Everyone else checks that their own screen shows the same six, in the same order. If they match, everyone taps **They match**. If anyone sees different words, everyone taps **They differ** and you start again from the beginning. Do not keep a secret that did not match.

If someone joined late and is still scanning at step 2 after you reached step 3, your animation is one tap away: **Someone still needs my reveal**.

## After

- Each device keeps the shared secret and nothing else. The random bytes each person contributed are erased.
- Each device shows a label next to the name: **CSPRNG** if anyone used only their phone's own randomness, **TRNG** if every member added hardware randomness. It says where the randomness came from. It is not a warning.
- The group can now send messages and chat. Nothing about the ceremony has to happen again until the secret runs low, and the app warns you when it does.

## If something goes wrong

- **Someone scanned a code for their own seat.** Two people picked the same seat. One of them leaves the ceremony from the menu and joins again with a different seat.
- **A phone reloaded or the app was closed.** That person's ceremony is gone. Everyone starts again from the beginning. The ceremony lives only in memory until the check words match.
- **The ceremony stopped on its own.** A reveal did not match its commitment. Nothing was kept. Start again. If it happens twice, one device may be faulty.
- **No camera.** Every scan has a paste fallback in the menu. Copy the code as text on one device and paste it on the other, over any channel you like. The codes are not secret.

## Badges

The hardware badge replaces scanning with stacking. Put the cards in a pile: magnets line each one up on the one below, and the pile passes the codes through the pads. The bottom card names the circle. Press B on the top card, wait for the buzz, lift your card off, and read the six words from its screen. The steps and the words are the same as on a phone; the seats come from the pile, so nobody says a number out loud. This section will grow once the first ceremonies have run on real badges.
