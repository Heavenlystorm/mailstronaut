# Native Nix configuration target for P02; the scaffold does not read it yet.
# Home Manager can link this file directly. Credentials never belong here.
{
  schemaVersion = 1;
  managed = true;
  locale = "en-GB";
  timeZone = "Europe/Paris";
  appearance = "system";
  background = {
    enabled = false;
    autostart = false;
  };
  notifications = {
    enabled = true;
    showPreview = false;
  };
  priority = {
    enabled = false;
    modelDirectory = null;
    modelRevision = null;
    languages = [
      "en"
      "fr"
      "it"
    ];
    defaultTtlHours = 72;
    maxTtlHours = 168;
  };
  accounts = [ ];
}
