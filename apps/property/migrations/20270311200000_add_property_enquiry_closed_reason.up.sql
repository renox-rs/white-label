-- BL-SAL-27: an enquiry closed as duplicate, spam or invalid is left out of
-- the campaign leads, as a wl-sales lead is. Empty for an open enquiry.
ALTER TABLE "property_enquiries" ADD COLUMN closed_reason TEXT;
