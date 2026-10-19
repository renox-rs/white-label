-- JSON list of the proof methods a statutory type accepts (#59 A5).
ALTER TABLE "wl_notify_types" ADD COLUMN proof_methods TEXT NOT NULL
    DEFAULT '["courier","registered_mail","hand_delivery","call"]';
