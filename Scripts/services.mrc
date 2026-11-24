menu channel {
  -
  services
  .re-join
  ..re-join $chan: hop
  ..-
  ..re-join alle channel: rejoinall
  .-
  .chan-serv
  ..help:.msg ChanServ help
  ..-
  ..xop
  ...xop einschalten:.cs set # xop on
  ...-
  ...xop ausschalten:.cs set # xop off
  ..-
  ..register $chan :.msg ChanServ register # $$?*="Channel Passwort?" $$?="Channel Beschreibung?"
  ..identify $chan :.msg ChanServ identify # $$?*="Channel Passwort?"
  ..drop $chan :if ($$?!="Soll der Channel wirklich gelöscht werden?" == $true) { .msg ChanServ drop # }
  ..-
  ..[info] $chan :.msg ChanServ INFO # ALL
  ..-
  ..prefix in $chan:prfx
  ..-
  ..setzen..
  ...founder:.msg ChanServ SET # founder $$?="Neuer Founder?"
  ...successor:.msg ChanServ SET # SUCCESSOR $$?="Neuer Successor?"
  ...passwort:.msg ChanServ SET # password $$?*="Neues Passwort?"
  ...beschreibung:.msg ChanServ SET # Desc $$?="Channel Beschreibung?"
  ...url:.msg ChanServ SET $chan URL $$?="Welche URL?"
  ...eMail:.msg ChanServ SET # EMAIL $$?="E-Mail Addresse?"
  ...topic setzen:{ set %topic.chan $chan | dialog -m topic topic }
  ...MLock:.msg ChanServ SET # Mlock $$?="Welche Modes? z.B. +tn"
  ...user begrüssung: .msg chanserv set # entrymsg $$?"womit soll der user beim betreten des channels begrüsst werden (per notiz) ?"
  ...topic schützen
  ....[ON]:.msg ChanServ SET # keeptopic on
  ....[OFF]:.msg ChanServ SET # keeptopic off
  ...topic sperren
  ....[ON]:.msg ChanServ SET # TopicLock On
  ....[OFF]:.msg ChanServ SET # TopicLock Off
  ...Private
  ....[ON]:.msg ChanServ SET # Private On
  ....[OFF]:.msg ChanServ SET # Private Off
  ...SecureOPs
  ....[ON]:.msg ChanServ SET # SecureOps On
  ....[OFF]:.msg ChanServ SET # SecureOps Off
  ...Restricted
  ....[ON]:.msg ChanServ SET # restricted On
  ....[OFF]:.msg ChanServ SET # restricted Off
  ...Secure
  ....[ON]:.msg ChanServ SET # Secure On
  ....[OFF]:.msg ChanServ SET # Secure Off
  ...OpNotice
  ....[ON]:.msg ChanServ SET # OPNOTICE On
  ....[OFF]:.msg ChanServ SET # OPNOTICE Off
  ...Securefounder
  ....[ON]:.msg ChanServ SET # SECUREFOUNDER ON
  ....[OFF]:.msg ChanServ SET # SECUREFOUNDER OFF
  ...SignKick
  ....[ON]:.msg ChanServ SET # SIGNKICK ON
  ....[OFF]:.msg ChanServ SET # SIGNKICK OFF
  ....Level:.msg ChanServ SET # SIGNKICK LEVEL $$?="Welches Levels?"
  ..Access
  ...List Access:.msg ChanServ access # list
  ...-
  ...Add Nick -2:.msg ChanServ access # add $$?="Welcher Nick?" -2
  ...Add Nick -1:.msg ChanServ access # add $$?="Welcher Nick?" -1
  ...Add Nick 1:.msg ChanServ access # add $$?="Welcher Nick?" 1
  ...Add Nick 3:.msg ChanServ access # add $$?="Welcher Nick?" 3
  ...Add Nick 4:.msg ChanServ access # add $$?="Welcher Nick?" 4
  ...Add Nick 5:.msg ChanServ access # add $$?="Welcher Nick?" 5
  ...Add Nick 10:.msg ChanServ access # add $$?="Welcher Nick?" 10
  ...Add Nick Level?:.msg ChanServ access # add $$1 $$?="Welches Levels?"
  ...-
  ...Del Nick:.msg ChanServ access # del $$?="Welcher Nick?"
  ...Del Entry-Num:.msg ChanServ access # del $$?="Eintrag Nummer?"
  ...Del List:.msg ChanServ access # del 1-1000
  ...-
  ...Clear List:.msg ChanServ access # CLEAR
  ..Levels
  ...Levels
  ....List:.msg ChanServ LEVELS # LIST
  ....ReSET:.msg ChanServ LEVELS # RESET
  ....-
  ....Desc:.msg ChanServ HELP LEVELS DESC
  ...Change Levels
  ....AutoOp:.msg ChanServ LEVELS # SET AUTOOP $$?="Neues Level? - Standart ist 5"
  ....AutoHalfop:.msg ChanServ LEVELS # SET AUTOHALFOP $$?="Neues Level? - Standart ist 4"
  ....AutoVoice:.msg ChanServ LEVELS # SET AUTOVOICE $$?="Neues Level? - Standart ist 3"
  ....AutoDeOp:.msg ChanServ LEVELS # SET AUTODEOP $$?="Neues Level? - Standart ist -1"
  ....Autoprotect:.msg ChanServ LEVELS # SET AUTOPROTECT $$?="Neues Level? - Standart ist 10"
  ....NoJoin:.msg ChanServ LEVELS # SET NOJOIN $$?="Neues Level? - Standart ist -2"
  ....Signkick:.msg ChanServ LEVELS # SET SIGNKICK $$?="Neues Level? - Standart ist DISABLED!"
  ....Invite:.msg ChanServ LEVELS # SET INVITE $$?="Neues Level? - Standart ist 5"
  ....AKick:.msg ChanServ LEVELS # SET AKICK $$?="Neues Level? - Standart ist 10"
  ....Set:.msg ChanServ LEVELS # SET SET $$?="Neues Level? - Standart ist DISABLED!"
  ....Clear:.msg ChanServ LEVELS # SET CLEAR $$?="Neues Level? - Standart ist DISABLED!"
  ....Getkey:.msg ChanServ LEVELS # SET GETKEY $$?="Neues Level? - Standart ist 5"
  ....HalfOp:.msg ChanServ LEVELS # SET HALFOP $$?="Neues Level? - Standart ist 5"
  ....HalfOpMe:.msg ChanServ LEVELS # SET HALFOPME $$?="Neues Level? - Standart ist 4"
  ....Kick:.msg ChanServ LEVELS # SET KICK $$?="Neues Level? - Standart ist 5"
  ....KickMe:.msg ChanServ LEVELS # SET KICKME $$?="Neues Level? - Standart ist 5"
  ....Invite:.msg ChanServ LEVELS # SET INVITE $$?="Neues Level? - Standart ist 5"
  ....Unban:.msg ChanServ LEVELS # SET UNBAN $$?="Neues Level? - Standart ist 5"
  ....OpDeOp:.msg ChanServ LEVELS # SET OPDEOP $$?="Neues Level? - Standart ist 5"
  ....OpDeOpMe:.msg ChanServ LEVELS # SET OPDEOPME $$?="Neues Level? - Standart ist 5"
  ....Protect:.msg ChanServ LEVELS # SET PROTECT $$?="Neues Level? - Standart ist DISABLED!"
  ....ProtectMe:.msg ChanServ LEVELS # SET PROTECTME $$?="Neues Level? - Standart ist 10"
  ....Voice:.msg ChanServ LEVELS # SET VOICE $$?="Neues Level? - Standart ist 5"
  ....VoiceMe:.msg ChanServ LEVELS # SET VOICEME $$?="Neues Level? - Standart ist 3"
  ....Acc-List:.msg ChanServ LEVELS # SET ACC-LIST $$?="Neues Level? - Standart ist 0"
  ....Acc-Change:.msg ChanServ LEVELS # SET ACC-CHANGE $$?="Neues Level? - Standart ist 1"
  ....Memo:.msg ChanServ LEVELS # SET MEMO $$?="Neues Level? - Standart ist 10"
  ....Assign:.msg ChanServ LEVELS # SET Assign $$?="Neues Level? - Standart ist DISABLED!"
  ....Badwords:.msg ChanServ LEVELS # SET Badwords $$?="Neues Level? - Standart ist 10"
  ....NoKick:.msg ChanServ LEVELS # SET Nokick $$?="Neues Level? - Standart ist 1"
  ....Fantasia:.msg ChanServ LEVELS # SET Fantasia $$?="Neues Level? - Standart ist 3"
  ....Greet:.msg ChanServ LEVELS # SET GREET $$?="Neues Level? - Standart ist 5"
  ....Say/Act:.msg ChanServ LEVELS # SET Say $$?="Neues Level? - Standart ist 5"
  ...Disable Levels
  ....AutoOp:.msg ChanServ LEVELS # DIS AUTOOP
  ....AutoHalfop:.msg ChanServ LEVELS # DIS AUTOHALFOP
  ....AutoVoice:.msg ChanServ LEVELS # DIS AUTOVOICE
  ....AutoDeOp:.msg ChanServ LEVELS # DIS AUTODEOP
  ....Autoprotect:.msg ChanServ LEVELS # DIS AUTOPROTECT
  ....NoJoin:.msg ChanServ LEVELS # DIS NOJOIN
  ....Signkick:.msg ChanServ LEVELS # DIS SIGNKICK
  ....Invite:.msg ChanServ LEVELS # DIS INVITE
  ....AKick:.msg ChanServ LEVELS # DIS AKICK
  ....Set:.msg ChanServ LEVELS # DIS SET
  ....Clear:.msg ChanServ LEVELS # DIS CLEAR
  ....Getkey:.msg ChanServ LEVELS # DIS GETKEY
  ....HalfOp:.msg ChanServ LEVELS # DIS HALFOP
  ....HalfOpMe:.msg ChanServ LEVELS # DIS HALFOPME
  ....Kick:.msg ChanServ LEVELS # DIS KICK
  ....KickMe:.msg ChanServ LEVELS # DIS KICKME
  ....Invite:.msg ChanServ LEVELS # DIS INVITE
  ....Unban:.msg ChanServ LEVELS # DIS UNBAN
  ....OpDeOp:.msg ChanServ LEVELS # DIS OPDEOP
  ....OpDeOpMe:.msg ChanServ LEVELS # DIS OPDEOPME
  ....Protect:.msg ChanServ LEVELS # DIS PROTECT
  ....ProtectMe:.msg ChanServ LEVELS # DIS PROTECTME
  ....Voice:.msg ChanServ LEVELS # DIS VOICE
  ....VoiceMe:.msg ChanServ LEVELS # DIS VOICEME
  ....Acc-List:.msg ChanServ LEVELS # DIS ACC-LIST
  ....Acc-Change:.msg ChanServ LEVELS # DIS ACC-CHANGE
  ....Memo:.msg ChanServ LEVELS # DIS MEMO
  ....Assign:.msg ChanServ LEVELS # DIS Assign
  ....Badwords:.msg ChanServ LEVELS # DIS Badwords
  ....NoKick:.msg ChanServ LEVELS # DIS Nokick
  ....Fantasia:.msg ChanServ LEVELS # DIS Fantasia
  ....Greet:.msg ChanServ LEVELS # DIS GREET
  ....Say/Act:.msg ChanServ LEVELS # DIS Say
  ..Akick
  ...Add:.msg ChanServ akick # Add $$?="Welcher Nick? or Mask" $$?="Grund?"
  ...Del:.msg ChanServ akick # Del $$?="Welcher Nick? or Mask"
  ...-
  ...List:.msg ChanServ akick # List
  ...View:.msg ChanServ akick # VIEW
  ...-
  ...Enforce:.msg ChanServ akick # Enforce
  ...-
  ...Clear:.msg ChanServ akick # CLEAR
  ..Clear
  ...Modes:.msg ChanServ CLEAR # MODES
  ...Bans:.msg ChanServ CLEAR # BANS
  ...Op:.msg ChanServ CLEAR # OPS
  ...Halfop:.msg ChanServ CLEAR # HALFOPS
  ...Voices:.msg ChanServ CLEAR # VOICES
  ...Users:.msg ChanServ CLEAR # USERS
  ..-
  .NickServ
  ..Help:.msg NickServ help
  ..-
  ..Register:.msg NickServ register $$?*="Passwort?" $$?="E-Mail Addresse?"
  ..Identify:.msg NickServ identify $$?*="Passwort?"
  ..Group:.msg NickServ GROUP $$?="Welcher Nick?" $$?*="Passwort?"
  ..Drop:if ($$?!="Soll der Nick wirklich gelöscht werden?" == $true) { .msg NickServ drop }
  ..Recover:.msg NickServ recover $$?="Welcher Nick?"
  ..Release:.msg NickServ release $$?="Welcher Nick?"
  ..Ghost:.msg NickServ ghost $$?="Welcher Nick?" $$?*="Passwort?"
  ..Info All:.msg NickServ info $$?="Welcher Nick?" all
  ..Status:.msg NickServ Status $$?="Welcher Nick?"
  ..List:.msg NickServ list $$?="Welche Maske?"
  ..Set
  ...eMail:.msg NickServ SET Email $$?="E-Mail Addresse?"
  ...Url:.msg NickServ SET URL $$?="Welche URL?"
  ...ICQ:.msg NickServ SET ICQ $$?="Welche UIN?"
  ...Password:.msg NickServ SET Password $$?*="Neues Passwort?"
  ...Greet:.msg NickServ SET GREET $$?="Message?"
  ...Kill
  ....On:.msg NickServ SET kill On
  ....Off:.msg NickServ SET kill Off
  ....Quick:.msg NickServ SET kill Quick
  ....Immed:.msg NickServ SET kill immed
  ...Secure
  ....On:.msg NickServ SET secure On
  ....Off:.msg NickServ SET secure Off
  ...Hide
  ....eMail
  .....On:.msg NickServ SET Hide EMAIL ON
  .....Off:.msg NickServ SET Hide EMAIL Off
  ....UserMask
  .....On:.msg NickServ SET Hide USERMASK ON
  .....Off:.msg NickServ SET Hide USERMASK Off
  ....Quit
  .....On:.msg NickServ SET Hide QUIT ON
  .....Off:.msg NickServ SET Hide QUIT Off
  ...Private
  ....On:.msg NickServ SET PRIVATE ON
  ....Off:.msg NickServ SET PRIVATE Off
  ..Access
  ...Add:.msg NickServ Access Add $$?="Welche Maske?"
  ...Del:.msg NickServ Access Del $$?="Welche Maske?"
  ...List:.msg NickServ Access list
  .BotServ
  ..Help:.msg BotServ help
  ..-
  ..List:.msg BotServ botlist
  ..Info:.msg BotServ info $$?="channel / botnick"
  ..-
  ..Assign:.msg BotServ assign # $$?="botnick"
  ..Unassign:if ($input(Soll der Bot wirklich entfernt werden?,q,botserv) == $true) { .msg BotServ unassign $chan }
  ..-
  ..Say:.msg BotServ Say # $$?="text"
  ..Action:.msg BotServ act # $$?="Action text"
  ..-
  ..Set
  ...Dont Kick Ops
  ....On:.msg BotServ SET # DONTKICKOPS ON
  ....Off:.msg BotServ SET # DONTKICKOPS OFF
  ...Dont Kick Voices
  ....On:.msg BotServ SET # DONTKICKVOICES ON
  ....Off:.msg BotServ SET # DONTKICKVOICES OFF
  ...Greet
  ....On:.msg BotServ SET # GREET ON
  ....Off:.msg BotServ SET # GREET OFF
  ...Fantasy
  ....On:.msg BotServ SET # FANTASY ON
  ....Off:.msg BotServ SET # FANTASY OFF
  ...Symbiosis
  ....On:.msg BotServ SET # SYMBIOSIS ON
  ....Off:.msg BotServ SET # SYMBIOSIS OFF
  ..Kicks
  ...Bolds
  ....On:.msg BotServ kick # BOLDS ON $$?="x kicks vorm ban"
  ....Off:.msg BotServ kick # BOLDS OFF
  ...Badwords
  ....On:.msg BotServ kick # BADWORDS ON $$?="x kicks vorm ban"
  ....Off:.msg BotServ kick # BADWORDS OFF
  ...Caps
  ....On:.msg BotServ kick # CAPS ON $$?="x kicks vorm ban" $$?="min caps" $$?="prozent caps"
  ....Off:.msg BotServ kick # CAPS OFF
  ...Colors
  ....On:.msg BotServ kick # COLORS ON $$?="x kicks vorm ban"
  ....Off:.msg BotServ kick # COLORS OFF
  ...Flood
  ....On:.msg BotServ kick # FLOOD ON $$?="x mal kicked vorm ban" $$?="Lines" $$?="sec"
  ....Off:.msg BotServ kick # FLOOD OFF
  ...Repeat
  ....On:.msg BotServ kick # REPEAT ON $$?="ban nach x kicks" $$?="max repeats"
  ....Off:.msg BotServ kick # REPEAT OFF
  ...Reverses
  ....On:.msg BotServ kick # REVERSES ON
  ....Off:.msg BotServ kick # REVERSES OFF
  ...Underlines
  ....On:.msg BotServ kick # UNDERLINES ON
  ....Off:.msg BotServ kick # UNDERLINES OFF
  ..Badwords
  ...Add
  ....Single:.msg BotServ BADWORDS # add $$?="Badword?" SINGLE
  ....Start:.msg BotServ BADWORDS # add $$?="Badword?" START
  ....End:.msg BotServ BADWORDS # add $$?="Badword?" END
  ...Del
  ....word:.msg BotServ BADWORDS # DEL $$?="word?"
  ....entry-num:.msg BotServ BADWORDS # DEL $$?="entry-num?"
  ....list:.msg BotServ BADWORDS # DEL $$?="List?"
  ...List
  ....List:.msg BotServ BADWORDS # LIST $$?="List?"
  ....Mask:.msg BotServ BADWORDS # LIST $$?="Mask?"
  ....All:.msg BotServ BADWORDS # LIST 1-5000
  .MemoServ
  ..Help:.msg MemoServ help
  ..-
  ..Send
  ...To Nick:.msg MemoServ send $$?="Welcher Nick?" $$?="Message?"
  ...To Channel:.msg MemoServ send $$?="Welcher Channel?" $$?="Message?"
  ..Cancel
  ...To Nick:.msg MemoServ CANCEL $$?="Welcher Nick?"
  ...To Channel:.msg MemoServ CANCEL $$?="Welcher Channel?"
  ..Read
  ...Nick Mesages 
  ....Last:.msg MemoServ Read LAST
  ....New:.msg MemoServ Read NEW
  ...Channel Mesages
  ....Last:.msg MemoServ Read $$?="Welcher Channel?" LAST
  ....New:.msg MemoServ Read $$?="Welcher Channel?" NEW
  ..List
  ...Nick Mesages
  ....All:.msg MemoServ List
  ....New:.msg MemoServ List NEW
  ...Channel Mesages
  ....All:.msg MemoServ List $$?="Welcher Channel?"
  ....New:.msg MemoServ List $$?="Welcher Channel?" NEW  
  ..Del
  ...Nick Mesages
  ....Number:.msg MemoServ Del $$?="Welche Nummer?"
  ....All:.msg MemoServ Del ALL
  ...Channel Mesages
  ....Number:.msg MemoServ Del $$?="Welcher Channel?" $$?="Welche Nummer?"
  ....All:.msg MemoServ Del $$?="Welcher Channel?" ALL  
  ..-
  ..Notify
  ...On:.msg MemoServ SET notify on
  ...Off:.msg MemoServ SET notify off
  ...Logon:.msg MemoServ SET notify logon
  ...New:.msg MemoServ SET notify new
  ..-
  ..Limit
  ...You
  ....Limit to number of Mesages:.msg MemoServ SET LIMIT $$?="Anzahl maximaler Memos?"
  ....None Limit:.msg MemoServ SET LIMIT NONE
  ...Channel
  ....Limit to number of Mesages:.msg MemoServ SET LIMIT $$?="Welcher Channel?" $$?="Anzahl maximaler Memos?"
  ....None Limit:.msg MemoServ SET LIMIT $$?="Welcher Channel?" NONE
  ....Hard:.msg MemoServ SET LIMIT $$?="Welcher Channel?" HARD
}





Menu MenuBar {
  -
  Services
  .re-join
  ..re-join $chan: /hop
  ..-
  ..re-join alle channel: /rejoinall
  .-
  .chanserv 
  ..help:.msg chanserv help
  ..-
  ..register:.msg chanserv register $$?="welcher channel?" $$?*="channel passwort?" $$?="channel beschreibung?"
  ..identify:.msg chanserv identify $$?="welcher channel?" $$?*="channel passwort?"
  ..drop:if ($$?!="soll der channel wirklich gelöscht werden?" == $true) { .msg ChanServ drop $$?="welcher channel?" }
  ..info:.msg ChanServ INFO $$?="welcher channel? " ALL
  ..getkey:.msg ChanServ GETKEY $$?="welcher channel?"
  ..setzen
  ...founder:.msg ChanServ SET $$?="welcher channel?" founder $$?="neuer founder?"
  ...successor:.msg ChanServ SET $$?="welcher channel?" SUCCESSOR $$?="neuer successor?"
  ...password:.msg ChanServ SET $$?="welcher channel?" password $$?*="neues passwort?"
  ...desc:.msg ChanServ SET $$?="welcher channel?" Desc $$?="channel beschreibung?"
  ...URL:.msg ChanServ SET $$?="welcher channel?" URL $$?="welche URL?"
  ...eMail:.msg ChanServ SET $$?="welcher channel?" EMAIL $$?="eMail addresse?"
  ...topic setzen:{ set %topic.chan $chan | dialog -m topic topic }
  ...Mlock:.msg ChanServ SET $$?="welcher channel?" Mlock $$?="welche modes? z.B. +tn"
  ...user begrüssung: /msg chanserv set # entrymsg $$?"womit soll der user beim betreten des channels begrüsst werden (per notiz) ?"
  ...Keeptopic
  ....On:.msg ChanServ SET $$?="Welcher Channel? Name" keeptopic on
  ....Off:.msg ChanServ SET $$?="Welcher Channel? Name" keeptopic off
  ...TopicLock
  ....On:.msg ChanServ SET $$?="Welcher Channel? Name" TopicLock On
  ....Off:.msg ChanServ SET $$?="Welcher Channel? Name" TopicLock Off
  ...Private
  ....On:.msg ChanServ SET $$?="Welcher Channel? " Private On
  ....Off:.msg ChanServ SET $$?="Welcher Channel? " Private Off
  ...SecureOPs
  ....On:.msg ChanServ SET $$?="Welcher Channel? " SecureOps On
  ....Off:.msg ChanServ SET $$?="Welcher Channel? " SecureOps Off
  ...Restricted
  ....On:.msg ChanServ SET $$?="Welcher Channel? " restricted On
  ....Off:.msg ChanServ SET $$?="Welcher Channel? " restricted Off
  ...Secure
  ....On:.msg ChanServ SET $$?="Welcher Channel? " Secure On
  ....Off:.msg ChanServ SET $$?="Welcher Channel? " Secure Off
  ...OPNotice
  ....On:.msg ChanServ SET $$?="Welcher Channel? " OPNOTICE On
  ....Off:.msg ChanServ SET $$?="Welcher Channel? " OPNOTICE Off
  ...Bantype
  ....*!user@host:.msg ChanServ SET $$?="Welcher Channel? " BANTYPE 0
  ....*!*user@host:.msg ChanServ SET $$?="Welcher Channel? " BANTYPE 1
  ....*!*@host:.msg ChanServ SET $$?="Welcher Channel? " BANTYPE 3
  ....*!*user@*.domain:.msg ChanServ SET $$?="Welcher Channel? " BANTYPE 3
  ...Securefounder
  ....On:.msg ChanServ SET $$?="Welcher Channel? " SECUREFOUNDER ON
  ....Off:.msg ChanServ SET $$?="Welcher Channel? " SECUREFOUNDER OFF
  ...SignKick
  ....On:.msg ChanServ SET $$?="Welcher Channel? " SIGNKICK ON
  ....Off:.msg ChanServ SET $$?="Welcher Channel? " SIGNKICK OFF
  ....Level:.msg ChanServ SET $$?="Welcher Channel? " SIGNKICK LEVEL $$?="Welches Levels?"
  ..Access
  ...List Access:.msg ChanServ access $$?="Welcher Channel? " list
  ...-
  ...Add Nick -2:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" -2
  ...Add Nick -1:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" -1
  ...Add Nick 1:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" 1
  ...Add Nick 3:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" 3
  ...Add Nick 4:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" 4
  ...Add Nick 5:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" 5
  ...Add Nick 10:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" 10
  ...Add Nick ?:.msg ChanServ access $$?="Welcher Channel? " add $$?="Welcher Nick?" $$?="Welches Levels?"
  ...-
  ...Del Nick:.msg ChanServ access $$?="Welcher Channel? " del $$?="Welcher Nick?" 
  ...Del Entry-Num:.msg ChanServ access $$?="Welcher Channel? " del $$?="Eintrag Nummer?" 
  ...Del List:.msg ChanServ access $$?="Welcher Channel? " del 1-1000
  ..Levels
  ...Levels List:.msg ChanServ LEVELS $$?="Welcher Channel? " LIST
  ...Levels ReSET:.msg ChanServ LEVELS $$?="Welcher Channel? " RESET
  ...-
  ...Change Levels
  ....AutoOp:.msg ChanServ LEVELS $$?="Welcher Channel? " SET AUTOOP $$?="Neues Level? - Standart ist 5"
  ....AutoHalfop:.msg ChanServ LEVELS $$?="Welcher Channel? " SET AUTOHALFOP $$?="Neues Level? - Standart ist 4"
  ....AutoVoice:.msg ChanServ LEVELS $$?="Welcher Channel? " SET AUTOVOICE $$?="Neues Level? - Standart ist 3"
  ....AutoDeOp:.msg ChanServ LEVELS $$?="Welcher Channel? " SET AUTODEOP $$?="Neues Level? - Standart ist -1"
  ....Autoprotect:.msg ChanServ LEVELS $$?="Welcher Channel? " SET AUTOPROTECT $$?="Neues Level? - Standart ist 10"
  ....NoJoin:.msg ChanServ LEVELS $$?="Welcher Channel? " SET NOJOIN $$?="Neues Level? - Standart ist -2"
  ....Signkick:.msg ChanServ LEVELS $$?="Welcher Channel? " SET SIGNKICK $$?="Neues Level? - Standart ist DISABLED!"
  ....Invite:.msg ChanServ LEVELS $$?="Welcher Channel? " SET INVITE $$?="Neues Level? - Standart ist 5"
  ....AKick:.msg ChanServ LEVELS $$?="Welcher Channel? " SET AKICK $$?="Neues Level? - Standart ist 10"
  ....Set:.msg ChanServ LEVELS $$?="Welcher Channel? " SET SET $$?="Neues Level? - Standart ist DISABLED!"
  ....Clear:.msg ChanServ LEVELS $$?="Welcher Channel? " SET CLEAR $$?="Neues Level? - Standart ist DISABLED!"
  ....Getkey:.msg ChanServ LEVELS $$?="Welcher Channel? " SET GETKEY $$?="Neues Level? - Standart ist 5"
  ....HalfOp:.msg ChanServ LEVELS $$?="Welcher Channel? " SET HALFOP $$?="Neues Level? - Standart ist 5"
  ....HalfOpMe:.msg ChanServ LEVELS $$?="Welcher Channel? " SET HALFOPME $$?="Neues Level? - Standart ist 4"
  ....Kick:.msg ChanServ LEVELS $$?="Welcher Channel? " SET KICK $$?="Neues Level? - Standart ist 5"
  ....KickMe:.msg ChanServ LEVELS $$?="Welcher Channel? " SET KICKME $$?="Neues Level? - Standart ist 5"
  ....Invite:.msg ChanServ LEVELS $$?="Welcher Channel? " SET INVITE $$?="Neues Level? - Standart ist 5"
  ....Unban:.msg ChanServ LEVELS $$?="Welcher Channel? " SET UNBAN $$?="Neues Level? - Standart ist 5"
  ....OpDeOp:.msg ChanServ LEVELS $$?="Welcher Channel? " SET OPDEOP $$?="Neues Level? - Standart ist 5"
  ....OpDeOpMe:.msg ChanServ LEVELS $$?="Welcher Channel? " SET OPDEOPME $$?="Neues Level? - Standart ist 5"
  ....Protect:.msg ChanServ LEVELS $$?="Welcher Channel? " SET PROTECT $$?="Neues Level? - Standart ist DISABLED!"
  ....ProtectMe:.msg ChanServ LEVELS $$?="Welcher Channel? " SET PROTECTME $$?="Neues Level? - Standart ist 10"
  ....Voice:.msg ChanServ LEVELS $$?="Welcher Channel? " SET VOICE $$?="Neues Level? - Standart ist 5"
  ....VoiceMe:.msg ChanServ LEVELS $$?="Welcher Channel? " SET VOICEME $$?="Neues Level? - Standart ist 3"
  ....Acc-List:.msg ChanServ LEVELS $$?="Welcher Channel? " SET ACC-LIST $$?="Neues Level? - Standart ist 0"
  ....Acc-Change:.msg ChanServ LEVELS $$?="Welcher Channel? " SET ACC-CHANGE $$?="Neues Level? - Standart ist 1"
  ....Memo:.msg ChanServ LEVELS $$?="Welcher Channel? " SET MEMO $$?="Neues Level? - Standart ist 10"
  ....Assign:.msg ChanServ LEVELS $$?="Welcher Channel? " SET Assign $$?="Neues Level? - Standart ist DISABLED!"
  ....Badwords:.msg ChanServ LEVELS $$?="Welcher Channel? " SET Badwords $$?="Neues Level? - Standart ist 10"  
  ....NoKick:.msg ChanServ LEVELS $$?="Welcher Channel? " SET Nokick $$?="Neues Level? - Standart ist 1"  
  ....Fantasia:.msg ChanServ LEVELS $$?="Welcher Channel? " SET Fantasia $$?="Neues Level? - Standart ist 3" 
  ....Greet:.msg ChanServ LEVELS $$?="Welcher Channel? " SET GREET $$?="Neues Level? - Standart ist 5" 
  ....Say/Act:.msg ChanServ LEVELS $$?="Welcher Channel? " SET Say $$?="Neues Level? - Standart ist 5"  
  ...Disable Levels
  ....AutoOp:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS AUTOOP 
  ....AutoHalfop:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS AUTOHALFOP  
  ....AutoVoice:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS AUTOVOICE   
  ....AutoDeOp:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS AUTODEOP   
  ....Autoprotect:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS AUTOPROTECT   
  ....NoJoin:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS NOJOIN   
  ....Signkick:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS SIGNKICK   
  ....Invite:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS INVITE 
  ....AKick:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS AKICK   
  ....Set:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS SET   
  ....Clear:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS CLEAR   
  ....Getkey:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS GETKEY 
  ....HalfOp:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS HALFOP 
  ....HalfOpMe:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS HALFOPME   
  ....Kick:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS KICK 
  ....KickMe:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS KICKME 
  ....Invite:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS INVITE 
  ....Unban:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS UNBAN 
  ....OpDeOp:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS OPDEOP 
  ....OpDeOpMe:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS OPDEOPME 
  ....Protect:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS PROTECT   
  ....ProtectMe:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS PROTECTME  
  ....Voice:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS VOICE 
  ....VoiceMe:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS VOICEME  
  ....Acc-List:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS ACC-LIST   
  ....Acc-Change:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS ACC-CHANGE   
  ....Memo:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS MEMO   
  ....Assign:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS Assign   
  ....Badwords:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS Badwords    
  ....NoKick:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS Nokick   
  ....Fantasia:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS Fantasia  
  ....Greet:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS GREET  
  ....Say/Act:.msg ChanServ LEVELS $$?="Welcher Channel? " DIS Say 
  ..Akick
  ...Add:.msg ChanServ akick $$?="Welcher Channel? " Add $$?="Welcher Nick?" $$?="Grund?"
  ...Del:.msg ChanServ akick $$?="Welcher Channel? " Del $$?="Welcher Nick?" 
  ...- 
  ...List:.msg ChanServ akick $$?="Welcher Channel? " List
  ...View:.msg ChanServ akick $$?="Welcher Channel? " VIEW
  ...-
  ...Clear:.msg ChanServ akick $$?="Welcher Channel? " CLEAR
  ..Clear
  ...Modes:/.msg ChanServ CLEAR $$?="Welcher Channel? " Modes
  ...Bans:.msg ChanServ CLEAR $$?="Welcher Channel? " Bans
  ...Op:.msg ChanServ CLEAR $$?="Welcher Channel? " Ops
  ...HalfOp:.msg ChanServ CLEAR $$?="Welcher Channel? " HalfOps
  ...Voices:.msg ChanServ CLEAR $$?="Welcher Channel? " Voices
  ...Users:.msg ChanServ CLEAR $$?="Welcher Channel? " Uusers
  ..-
  ..You
  ...UnBan:.msg ChanServ unban $$?="Welcher Channel? "
  ...Voice:.msg ChanServ voice $$?="Welcher Channel? "
  ...DeVoice:.msg ChanServ devoice $$?="Welcher Channel? "
  ...OP:.msg ChanServ op $$?="Welcher Channel? "
  ...DeOP:.msg ChanServ deop $$?="Welcher Channel? " 
  ...Protect:.msg ChanServ PROTECT $$?="Welcher Channel? "
  ...DeProtect:.msg ChanServ DEPROTECT $$?="Welcher Channel? "     
  ...Invite:.msg ChanServ INVITE $$?="Welcher Channel?"

  .NickServ
  ..help:.msg NickServ help
  ..-
  ..Register:.msg NickServ register $$?*="Passwort?" $$?="E-Mail Addresse?"
  ..Identify:.msg NickServ identify $$?*="Passwort?"
  ..Link:.msg NickServ link $$?="Welcher Nick?" $$?*="Passwort?"
  ..UnLink:.msg NickServ unlink $$?="Welcher Nick?" $$?*="Passwort?"
  ..Drop:if ($$?!="Soll der Nick wirklich gelöscht werden?" == $true) { .msg NickServ drop }
  ..Recover:.msg NickServ recover $$?="Welcher Nick?"
  ..Release:.msg NickServ release $$?="Welcher Nick?"
  ..Ghost:.msg NickServ ghost $$?="Welcher Nick?" $$?*="Passwort?"
  ..Info All:.msg NickServ info $$?="Welcher Nick?" all
  ..Status:.msg NickServ Status $$?="Welcher Nick?"
  ..List:.msg NickServ list $$?="Welche Maske?"
  ..Set
  ...eMail:.msg NickServ SET Email $$?="E-Mail Addresse?"
  ...Url:.msg NickServ SET URL $$?="Welche URL?"
  ...ICQ:.msg NickServ SET ICQ $$?="Welche UIN?"
  ...Password:.msg NickServ SET Password $$?*="Neues Passwort?"
  ...Greet:.msg NickServ SET GREET $$?="Message?"
  ...Kill
  ....On:.msg NickServ SET kill On
  ....Off:.msg NickServ SET kill Off
  ....Quick:.msg NickServ SET kill Quick
  ....Immed:.msg NickServ SET kill immed
  ...Secure
  ....On:.msg NickServ SET $me secure On
  ....Off:.msg NickServ SET $me secure Off
  ...Hide
  ....eMail
  .....On:.msg NickServ SET Hide EMAIL ON
  .....Off:.msg NickServ SET Hide EMAIL Off
  ....UserMask
  .....On:.msg NickServ SET Hide USERMASK ON
  .....Off:.msg NickServ SET Hide USERMASK Off
  ....Quit
  .....On:.msg NickServ SET Hide QUIT ON
  .....Off:.msg NickServ SET Hide QUIT Off
  ...Private
  ....On:.msg NickServ SET PRIVATE ON
  ....Off:.msg NickServ SET PRIVATE Off
  ..Access
  ...Add:.msg NickServ Access Add $$?="Welche Maske?"
  ...Del:.msg NickServ Access Del $$?="Welche Maske?"
  ...List:.msg NickServ Access list
  .BotServ
  ..help:.msg BotServ help
  ..-
  ..List:.msg BotServ botlist
  ..Info:.msg BotServ info $$?="channel / botnick"
  ..-
  ..Assign:.msg BotServ assign # $$?="botnick"
  ..Unassign:if ($$?!="Soll der Bot wirklich entfernt werden?" == $true) { .msg BotServ unassign # }
  ..-
  ..Say:.msg BotServ Say # $$?="text"
  ..Action:.msg BotServ act # $$?="Action text"
  ..-
  ..Set
  ...Dont Kick Ops
  ....On:.msg BotServ SET # DONTKICKOPS ON
  ....Off:.msg BotServ SET # DONTKICKOPS OFF
  ...Dont Kick Voices
  ....On:.msg BotServ SET # DONTKICKVOICES ON
  ....Off:.msg BotServ SET # DONTKICKVOICES OFF
  ...Greet
  ....On:.msg BotServ SET # GREET ON
  ....Off:.msg BotServ SET # GREET OFF
  ...Fantasy
  ....On:.msg BotServ SET # FANTASY ON
  ....Off:.msg BotServ SET # FANTASY OFF
  ...Symbiosis
  ....On:.msg BotServ SET # SYMBIOSIS ON
  ....Off:.msg BotServ SET # SYMBIOSIS OFF
  ..Kicks
  ...Bolds
  ....On:.msg BotServ kick # BOLDS ON $$?="x kicks vorm ban"
  ....Off:.msg BotServ kick # BOLDS OFF
  ...Badwords
  ....On:.msg BotServ kick # BADWORDS ON $$?="x kicks vorm ban"
  ....Off:.msg BotServ kick # BADWORDS OFF
  ...Caps
  ....On:.msg BotServ kick # CAPS ON $$?="x kicks vorm ban" $$?="min caps" $$?="prozent caps"
  ....Off:.msg BotServ kick # CAPS OFF
  ...Colors
  ....On:.msg BotServ kick # COLORS ON $$?="x kicks vorm ban"
  ....Off:.msg BotServ kick # COLORS OFF
  ...Flood
  ....On:.msg BotServ kick # FLOOD ON $$?="x mal kicked vorm ban" $$?="Lines" $$?="sec"
  ....Off:.msg BotServ kick # FLOOD OFF
  ...Repeat
  ....On:.msg BotServ kick # REPEAT ON $$?="ban nach x kicks" $$?="max repeats"
  ....Off:.msg BotServ kick # REPEAT OFF
  ...Reverses
  ....On:.msg BotServ kick # REVERSES ON
  ....Off:.msg BotServ kick # REVERSES OFF
  ...Underlines
  ....On:.msg BotServ kick # UNDERLINES ON
  ....Off:.msg BotServ kick # UNDERLINES OFF
  ..Badwords
  ...Add
  ....Single:.msg BotServ BADWORDS # add $$?="Badword?" SINGLE
  ....Start:.msg BotServ BADWORDS # add $$?="Badword?" START
  ....End:.msg BotServ BADWORDS # add $$?="Badword?" END
  ...Del
  ....word:.msg BotServ BADWORDS # DEL $$?="word?"
  ....entry-num:.msg BotServ BADWORDS # DEL $$?="entry-num?"
  ....list:.msg BotServ BADWORDS # DEL $$?="List?"
  ...List
  ....List:.msg BotServ BADWORDS # LIST $$?="List?"
  ....Mask:.msg BotServ BADWORDS # LIST $$?="Mask?"
  ....All:.msg BotServ BADWORDS # LIST 1-5000
  .MemoServ
  ..Help:.msg MemoServ help
  ..-
  ..Send
  ...To Nick:.msg MemoServ send $$?="Welcher Nick?" $$?="Message?"
  ...To Channel:.msg MemoServ send $$?="Welcher Channel?" $$?="Message?"
  ..Cancel
  ...To Nick:.msg MemoServ CANCEL $$?="Welcher Nick?"
  ...To Channel:.msg MemoServ CANCEL $$?="Welcher Channel?"
  ..Read
  ...Nick Mesages 
  ....Last:.msg MemoServ Read LAST
  ....New:.msg MemoServ Read NEW
  ...Channel Mesages
  ....Last:.msg MemoServ Read $$?="Welcher Channel?" LAST
  ....New:.msg MemoServ Read $$?="Welcher Channel?" NEW
  ..List
  ...Nick Mesages
  ....All:.msg MemoServ List
  ....New:.msg MemoServ List NEW
  ...Channel Mesages
  ....All:.msg MemoServ List $$?="Welcher Channel?"
  ....New:.msg MemoServ List $$?="Welcher Channel?" NEW  
  ..Del
  ...Nick Mesages
  ....Number:.msg MemoServ Del $$?="Welche Nummer?"
  ....All:.msg MemoServ Del ALL
  ...Channel Mesages
  ....Number:.msg MemoServ Del $$?="Welcher Channel?" $$?="Welche Nummer?"
  ....All:.msg MemoServ Del $$?="Welcher Channel?" ALL  
  ..-
  ..Notify
  ...On:.msg MemoServ SET notify on
  ...Off:.msg MemoServ SET notify off
  ...Logon:.msg MemoServ SET notify logon
  ...New:.msg MemoServ SET notify new
  ..-
  ..Limit
  ...You
  ....Limit to number of Mesages:.msg MemoServ SET LIMIT $$?="Anzahl maximaler Memos?"
  ....None Limit:.msg MemoServ SET LIMIT NONE
  ...Channel
  ....Limit to number of Mesages:.msg MemoServ SET LIMIT $$?="Welcher Channel?" $$?="Anzahl maximaler Memos?"
  ....None Limit:.msg MemoServ SET LIMIT $$?="Welcher Channel?" NONE
  ....Hard:.msg MemoServ SET LIMIT $$?="Welcher Channel?" HARD

  .HostServ
  ..Login:.msg HostServ VHost $$?="Login Nick?" $$?="Passwort?"
}






Menu NickList {
  -
  Services
  .User Aktionen
  ..Banschutz
  ...+e:.mode # +e $$1 $2 $3
  ...-e:.mode # -e $$1 $2 $3   
  .ChanServ
  ..Help:.msg ChanServ help
  ..-
  ..Op:.msg ChanServ op # $$1 $2 $3
  ..DeOp:.msg ChanServ deop # $$1 $2 $3
  ..-
  ..Voice:.msg ChanServ VOICE # $$1 $2 $3
  ..DeVoice:.msg ChanServ DEVOICE # $$1 $2 $3
  ..-
  ..Halfop:.msg ChanServ HALFOP # $$1 $2 $3
  ..DeHalfop:.msg ChanServ DEHALFOP # $$1 $2 $3
  ..-
  ..Protect:.msg ChanServ PROTECT # $$1 $2 $3
  ..DeProtect:.msg ChanServ DEPROTECT # $$1 $2 $3
  ..-
  ..Access
  ...List Access:.msg ChanServ access # list
  ...-
  ...Add $$1 -2:.msg ChanServ access # add $$1 -2
  ...Add $$1 -1:.msg ChanServ access # add $$1 -1
  ...Add $$1 1:.msg ChanServ access # add $$1 1
  ...Add $$1 3:.msg ChanServ access # add $$1 3
  ...Add $$1 4:.msg ChanServ access # add $$1 4
  ...Add $$1 5:.msg ChanServ access # add $$1 5
  ...Add $$1 10:.msg ChanServ access # add $$1 10
  ...Add $$1 ?:.msg ChanServ access # add $$1 $$?="Welches Level?"
  ...-
  ...Del $$1:.msg ChanServ access # del $$1
  ..Akick
  ...Add $$1:.msg ChanServ akick # Add $$1 $$?="Grund?"
  ...Del $$1:.msg ChanServ akick $actice Del $$1
  ...-
  ...List:.msg ChanServ akick # List
  ...View:.msg ChanServ akick # VIEW
  ...-
  ...Clear:.msg ChanServ akick # CLEAR
  ..-
  ..You
  ...UnBan:.msg ChanServ unban #
  ...Voice:.msg ChanServ voice #
  ...DeVoice:.msg ChanServ devoice #
  ...OP:.msg ChanServ op #
  ...DeOP:.msg ChanServ deop #
  ...Protect:.msg ChanServ PROTECT #
  ...DeProtect:.msg ChanServ DEPROTECT #
  ...Invite:.msg ChanServ INVITE #
  .NickServ
  ..Help:.msg NickServ help
  ..-
  ..Register:.msg NickServ register $$?*="Passwort?" $$?="E-Mail Addresse?"
  ..Identify:.msg NickServ identify $$?*="Passwort?"
  ..Group:.msg NickServ Group $$1 $$?*="Passwort?"
  ..Drop:if ($$?!="Soll der Nick wirklich gelöscht werden?" == $true) { .msg NickServ drop }
  ..Recover:.msg NickServ recover $$snicks
  ..Release:.msg NickServ release $$snicks
  ..Ghost:.msg NickServ ghost $$snicks $$?*="Passwort?"
  ..Info All:.msg NickServ info $$snicks all
  ..Status:.msg NickServ Status $$snicks
  ..List:.msg NickServ list $$?="Welche Maske?"
  ..Set
  ...eMail:.msg NickServ SET Email $$?="E-Mail Addresse?"
  ...Url:.msg NickServ SET URL $$?="Welche URL?"
  ...ICQ:.msg NickServ SET ICQ $$?="Welche UIN?"
  ...Password:.msg NickServ SET Password $$?*="Neues Passwort?"
  ...Greet:.msg NickServ SET GREET $$?="Message?"
  ...Kill
  ....On:.msg NickServ SET kill On
  ....Off:.msg NickServ SET kill Off
  ....Quick:.msg NickServ SET kill Quick
  ....Immed:.msg NickServ SET kill immed
  ...Secure
  ....On:.msg NickServ SET $me secure On
  ....Off:.msg NickServ SET $me secure Off
  ...Hide
  ....eMail
  .....On:.msg NickServ SET Hide EMAIL ON
  .....Off:.msg NickServ SET Hide EMAIL Off
  ....UserMask
  .....On:.msg NickServ SET Hide USERMASK ON
  .....Off:.msg NickServ SET Hide USERMASK Off
  ....Quit
  .....On:.msg NickServ SET Hide QUIT ON
  .....Off:.msg NickServ SET Hide QUIT Off
  ...Private
  ....On:.msg NickServ SET PRIVATE ON
  ....Off:.msg NickServ SET PRIVATE Off
  ..Access
  ...Add:.msg NickServ Access Add $$?="Welche Maske?"
  ...Del:.msg NickServ Access Del $$?="Welche Maske?"
  ...List:.msg NickServ Access list
  .MemoServ
  ..Help:.msg MemoServ help
  ..-
  ..Send
  ...To Nick:.msg MemoServ send $$1 $$?="Message?"
  ...To Channel:.msg MemoServ send $$?="Welcher Channel?" $$?="Message?"
  ..Cancel
  ...To Nick:.msg MemoServ CANCEL $$1
  ...To Channel:.msg MemoServ CANCEL $$?="Welcher Channel?"
  ..Read
  ...Nick Mesages 
  ....Last:.msg MemoServ Read LAST
  ....New:.msg MemoServ Read NEW
  ...Channel Mesages
  ....Last:.msg MemoServ Read $$?="Welcher Channel?" LAST
  ....New:.msg MemoServ Read $$?="Welcher Channel?" NEW
  ..List
  ...Nick Mesages
  ....All:.msg MemoServ List
  ....New:.msg MemoServ List NEW
  ...Channel Mesages
  ....All:.msg MemoServ List $$?="Welcher Channel?"
  ....New:.msg MemoServ List $$?="Welcher Channel?" NEW  
  ..Del
  ...Nick Mesages
  ....Number:.msg MemoServ Del $$?="Welche Nummer?"
  ....All:.msg MemoServ Del ALL
  ...Channel Mesages
  ....Number:.msg MemoServ Del $$?="Welcher Channel?" $$?="Welche Nummer?"
  ....All:.msg MemoServ Del $$?="Welcher Channel?" ALL  
  ..-
  ..Notify
  ...On:.msg MemoServ SET notify on
  ...Off:.msg MemoServ SET notify off
  ...Logon:.msg MemoServ SET notify logon
  ...New:.msg MemoServ SET notify new
  ..-
  ..Limit
  ...You
  ....Limit to number of Mesages:.msg MemoServ SET LIMIT $$?="Anzahl maximaler Memos?"
  ....None Limit:.msg MemoServ SET LIMIT NONE
  ...Channel
  ....Limit to number of Mesages:.msg MemoServ SET LIMIT $$?="Welcher Channel?" $$?="Anzahl maximaler Memos?"
  ....None Limit:.msg MemoServ SET LIMIT $$?="Welcher Channel?" NONE
  ....Hard:.msg MemoServ SET LIMIT $$?="Welcher Channel?" HARD
}


;_________________________________________

alias rejoinall {
  if ($chan(0) > 0) {
    var %i = 1
    while ($chan(%i)) { hop -c $chan(%i) | inc %i }
  }
}

;____________________________

alias prfx {

}
