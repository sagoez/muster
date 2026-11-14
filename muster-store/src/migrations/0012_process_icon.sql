-- The icon `muster view` draws a process with, as a Simple Icons slug. NULL
-- resolves one. No CHECK, so adding an icon needs no migration; a name the
-- binary does not know is refused on read.
ALTER TABLE processes ADD COLUMN icon TEXT;
