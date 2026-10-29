# Create a circle

A ceremony creates the secret your circle will share. Everyone takes part on their own device, then checks that the devices agree.

## Before you begin

- Gather two to eight phones or laptops in the same room.
- Open the [app](https://cahoots.network/app/) on each device.
- Turn up screen brightness and make sure there is enough light to scan.
- Keep each device open until the circle is saved. Reloading during the ceremony loses that device’s progress.

The QR codes carry encrypted contributions, not the shared secret in plaintext. You still need to check the words at the end.

## 1. Set up the group

One person selects **Start a ceremony** and chooses:

- **Name:** something everyone will recognize.
- **People in the room:** the number of participating devices, including the host.
- **Secret size:** choose **Small** for a first try. Larger sizes take longer to exchange.

Leave the hosted software member option under **Advanced** off for a circle made only of people. Enabling it gives a program a seat and a copy of the secret; everyone should agree before using it.

The host continues, keeps seat 1, and selects **Make my contribution**.

Everyone else selects **Join one**, scans the host’s code, and chooses a different seat number. Say your number aloud to avoid duplicates. Then select **Make my contribution**.

Hardware randomness is optional. If you have a suitable file, add it on your own device through **Add hardware randomness**. It is not needed for a first test.

## 2. Exchange commitments

The **Commit** screen shows a looping code and lists the members whose codes you still need.

Show your code to each member and scan theirs. Order does not matter. Your device advances when it has all the commitments.

A commitment prevents a member from changing their contribution after seeing everyone else’s.

## 3. Exchange reveals

The **Reveal** screen shows a longer sequence of codes. Scan every other member’s sequence.

Hold the devices still until the progress bar fills. This is usually the longest step. Your device advances when it has everyone’s reveal.

If someone still needs yours after you have moved on, use **Someone still needs my reveal** from the menu.

## 4. Exchange signatures

The **Sign** screen shows another short sequence. Show yours to everyone and scan theirs.

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
| Two people chose the same seat | One leaves the ceremony and rejoins with an unused number. |
| Scanning is slow | Increase brightness, reduce glare, and hold both devices still. Wait for the progress bar to finish. |
| A device has no camera | Use the copy and paste options in the menu to exchange the same data. |
| A device reloads or closes | Restart the ceremony on every device. Unsaved progress cannot be recovered. |
| A signature is rejected | Check whose code you are scanning and try again. |
| A reveal fails its commitment check | Restart. The device refuses to save the circle. |
| The six words differ | Discard the ceremony on every device and start again. |

Camera scanning has not been verified end to end. The browser’s automated checks exercise the paste path; use that if scanning fails.

For badges, see [Hardware](hardware.md). Multi-badge ceremonies remain development work; the phone guide does not establish that a hardware pile has been tested.
