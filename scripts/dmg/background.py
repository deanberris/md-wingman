# Draws the disk image window background (1x and @2x) in the website's colours.
# Run with Pillow and DejaVu Sans Mono: python3 scripts/dmg/background.py scripts/dmg
from PIL import Image, ImageDraw, ImageFont
import sys
W,H=660,400
out=sys.argv[1]
for scale,suffix in ((1,""),(2,"@2x")):
    s=scale
    im=Image.new("RGB",(W*s,H*s),"#fbfaf7")
    d=ImageDraw.Draw(im)
    mono=ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",13*s)
    monob=ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",15*s)
    fg="#1d1c1a"; muted="#6b6760"; rule="#dedad2"; accent="#2f5d8a"
    # top rule + title, TUI style
    d.text((32*s,26*s),"md-wingman",font=monob,fill=fg)
    d.line((32*s,54*s,(W-32)*s,54*s),fill=rule,width=s)
    # arrow between icon centres (165,190) and (495,190)
    y=190*s
    x0,x1=245*s,415*s
    d.line((x0,y,x1,y),fill=accent,width=2*s)
    d.polygon([(x1+12*s,y),(x1-2*s,y-8*s),(x1-2*s,y+8*s)],fill=accent)
    # caption
    cap="Drag md-wingman to Applications to install it."
    w=d.textlength(cap,font=mono)
    d.text(((W*s-w)/2,318*s),cap,font=mono,fill=muted)
    d.line((32*s,(H-40)*s,(W-32)*s,(H-40)*s),fill=rule,width=s)
    im.save(f"{out}/background{suffix}.png",optimize=True)
