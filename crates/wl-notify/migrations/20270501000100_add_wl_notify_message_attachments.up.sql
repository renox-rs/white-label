-- The file names of the documents an e-mail message carries (BL-NTF-10): the
-- message log lists them; a file sent as a signed link is marked "(link)".
-- The document references themselves travel in the message's data.
ALTER TABLE "wl_notify_messages" ADD COLUMN attachment_names TEXT;
