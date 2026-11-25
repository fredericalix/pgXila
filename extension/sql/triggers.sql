-- History triggers of the definition tables.
--
-- They are created last, after the extension has written the palette and the
-- system stacks: history starts empty in a new database and only ever records
-- what happens afterwards. This also keeps a restored history free of
-- collisions with rows the extension would otherwise have recorded itself.
--
-- They do not fire while a backup is being restored (see xila.restoring).

CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.palette
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.stack
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.stack_colour
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.background
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.action
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.widget
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.layout
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.key_binding
    FOR EACH ROW WHEN (NOT xila.restoring()) EXECUTE FUNCTION xila.record_definition_change();
