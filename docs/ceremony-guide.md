# Create a circle

A ceremony creates the secret your circle will share. Everyone takes part on their own device, then checks that the devices agree.

## Before you begin

- Gather two to eight phones or laptops in the same room.
- Open the [app](https://cahoots.network/app/) on each device.
- Turn up screen brightness and make sure there is enough light to scan.
- Keep each device open until the circle is saved. Reloading during the ceremony loses that device’s progress.

The QR codes carry encrypted contributions, not the shared secret in plaintext. You still need to check the words at the end.

## Two ways to hold a ceremony

- **Through the relay.** This is the default. The host shows one code. Everyone else scans it and picks a seat. Each joiner's screen then shows a small code: "Show this to the host". The host scans each one. When the host's screen says "Everyone is scanned", the host selects **Continue**. The rest goes through the relay, sealed under a key that is only in the host's code. Each device needs a network connection. Phones and laptops can take part together, whichever app they run.
- **Codes only.** Everyone shows and scans codes in turn, as in the steps below. No network is needed.

Both ways make the same kind of circle, and both end with the six words. Either way, everyone must be in the same room. The relay only carries bytes. The two scans show that everyone is present: each joiner scans the host's screen, and the host scans each joiner's screen. The host's code binds the host's own setup, and each joiner's code binds that joiner's. Someone who photographs the host's code from outside the room cannot get into the circle. The host never scans them, and the joiners only accept the host the code names. If a joiner's screen says "Seat 1 on the relay is not the host whose code you scanned", someone outside the room has the code. If the host's screen says "Seat N committed but was not scanned here" and nobody in the room is seat N, start over with a new code.

With **Through the relay**, steps 2 to 4 below happen on their own. The host's screen fills in as people join. Joiners wait. Then everyone goes to step 5.

## With badges in a pile

Badges can join a ceremony that a phone hosts through the relay. A laptop carries their part between the pile and the relay. The laptop holds no seat and never sees the secret.

1. On the host's phone, create the circle **Through the relay**. Count the badges in **People in the room**, and set **Cards in the pile** to the number of badges.
2. Stack the badges on the laptop, with the bottom badge on USB.
3. Copy the host's code to the laptop and run `cahoots-host bridge --serial <port> --offer <code>`. If the code is in a file, use `--offer-file <file>` instead.
4. The laptop prints a code for each badge. Scan each one with the host's phone. Phones that join show their own codes, as usual.
5. When the host's screen says "Everyone is scanned", select **Continue** on the host's phone.

The badges show the six words with everyone else. When the laptop says every seat has signed, the badges keep the circle.

## 1. Set up the group

One person selects **Create a circle** and chooses:

- **Name:** something everyone will recognize.
- **People in the room:** the number of participating devices, including the host.
- **How to hold the ceremony:** **Through the relay** or **Codes only**.
- **Secret size:** choose **Small** for a first try. Larger sizes take longer to exchange.

Leave the hosted software member option under **Advanced** off for a circle made only of people. Enabling it gives a program a seat and a copy of the secret; everyone should agree before using it.

The host continues, keeps seat 1, and selects **Make my contribution**.

Everyone else selects **Join a circle**, scans the host’s code, and chooses a different seat number. Say your number aloud to avoid duplicates. Then select **Make my contribution**.

Hardware randomness is optional. If you have a suitable file, add it on your own device through **Add hardware randomness**. It is not needed for a first test.

The steps 2 to 4 below are for **Codes only**.

## 2. Exchange commitments

The **Meet the group** screen shows a looping code and lists the members whose codes you still need.

Show your code to each member and scan theirs. Order does not matter. Your device advances when it has all the commitments.

A commitment prevents a member from changing their contribution after seeing everyone else’s.

## 3. Exchange reveals

The **Exchange secrets** screen shows a longer sequence of codes. Scan every other member’s sequence.

Hold the devices still until the progress bar fills. This is usually the longest step. Your device advances when it has everyone’s reveal.

Once you have every reveal, your device stays on this step and switches to showing your own code. Ask the others to scan it. Select **They have scanned my reveal** (or **Everyone has scanned my reveal**) only when they confirm they have it. Receiving their codes does not mean they received yours.

If someone still needs yours after you have moved on, use **Someone still needs my reveal**. The missing-seat list and received count describe what your device has collected, not what others have scanned.

## 4. Exchange signatures

The **Confirm members** screen shows another short sequence. Show yours to everyone and scan theirs.

Your device keeps showing its signature after it has collected the others. Wait for everyone to finish scanning before selecting **They have scanned it** or **Everyone has scanned it**.

Signatures record which seats belong to the circle.

## 5. Check the words and save

Every device shows six words. One person reads them aloud. Everyone checks all six words, in the same order, on their own screen.

- If every screen agrees, select **They match** on each device. The circle is saved.
- If any word differs, select **They differ · start over**. Everyone must restart. Do not keep that circle.

If someone still needs your signature, use **Someone still needs my signature** before saving.

## Send your first message

Open the saved circle and select **Send a message**. Pass its encrypted code or text to another member, who opens **Read a message**.

For ongoing chat, follow [Run a relay](relay.md). Saving a circle does not automatically connect it to a relay.

## Troubleshooting

| Problem | What to do |
|---|---|
| Two people chose the same seat | With **Codes only**, one leaves the ceremony and rejoins with an unused number. Through the relay, the device that picked second is told "Seat N was taken by someone else". That person leaves and joins again with a free seat. |
| Scanning is slow | Increase brightness, reduce glare, and hold both devices still. Wait for the progress bar to finish. |
| A device has no camera | Use the copy and paste options in the menu to exchange the same data. |
| A device reloads or closes | The ceremony is lost. Everyone starts over with a new code. Unsaved progress cannot be recovered. |
| A signature is rejected | Check whose code you are scanning and try again. |
| A reveal fails its commitment check | Restart. The device refuses to save the circle. |
| The six words differ | Discard the ceremony on every device and start again. |

Camera scanning has not been verified end to end. The browser’s automated checks exercise the paste path; use that if scanning fails.

For badges, see [Hardware](hardware.md). Multi-badge ceremonies remain development work; the phone guide does not establish that a hardware pile has been tested.
