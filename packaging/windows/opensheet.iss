; The Windows installer of OpenSheet (Inno Setup 6.3 or later).
;   iscc /DVersion=0.1.0 /DTarget=x86_64-pc-windows-msvc /DArch=x64compatible packaging\windows\opensheet.iss
; Arch is x64compatible for x86_64-pc-windows-msvc, arm64 for aarch64-pc-windows-msvc. It
; installs for the current user (no administrator rights), or for all users if chosen; it
; adds a Start menu entry, and registers the spreadsheet files so that they open with
; OpenSheet. Keep its list of extensions in step with CFBundleDocumentTypes
; (scripts/bundle.sh) and MimeType (packaging/linux/org.opensheet.OpenSheet.desktop).

#ifndef Version
  #error Pass /DVersion=x.y.z
#endif
#ifndef Target
  #define Target "x86_64-pc-windows-msvc"
#endif
#ifndef Arch
  #define Arch "x64compatible"
#endif

[Setup]
AppId={{09A3A789-74BE-4D6D-BA37-5B4EBF7F2E84}
AppName=OpenSheet
AppVersion={#Version}
AppVerName=OpenSheet {#Version}
AppPublisher=The OpenSheet contributors
AppPublisherURL=https://github.com/0xtlt/opensheet
AppSupportURL=https://github.com/0xtlt/opensheet/issues
AppUpdatesURL=https://github.com/0xtlt/opensheet/releases
VersionInfoVersion={#Version}
DefaultDirName={autopf}\OpenSheet
DefaultGroupName=OpenSheet
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
ChangesAssociations=yes
SetupIconFile=opensheet.ico
UninstallDisplayIcon={app}\opensheet.exe
UninstallDisplayName=OpenSheet
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\dist
OutputBaseFilename=opensheet-{#Target}-setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"

[CustomMessages]
english.AssociateFiles=Open spreadsheets (.xlsx, .xlsm, .xltx, .xltm, .csv, .tsv) with OpenSheet
french.AssociateFiles=Ouvrir les classeurs (.xlsx, .xlsm, .xltx, .xltm, .csv, .tsv) avec OpenSheet
spanish.AssociateFiles=Abrir las hojas de cálculo (.xlsx, .xlsm, .xltx, .xltm, .csv, .tsv) con OpenSheet
german.AssociateFiles=Tabellen (.xlsx, .xlsm, .xltx, .xltm, .csv, .tsv) mit OpenSheet öffnen
italian.AssociateFiles=Aprire i fogli di calcolo (.xlsx, .xlsm, .xltx, .xltm, .csv, .tsv) con OpenSheet
english.FileTypes=File types:
french.FileTypes=Types de fichiers :
spanish.FileTypes=Tipos de archivo:
german.FileTypes=Dateitypen:
italian.FileTypes=Tipi di file:
english.TypeWorkbook=Workbook
french.TypeWorkbook=Classeur
spanish.TypeWorkbook=Libro
german.TypeWorkbook=Arbeitsmappe
italian.TypeWorkbook=Cartella di lavoro
english.TypeMacroWorkbook=Macro-Enabled Workbook
french.TypeMacroWorkbook=Classeur prenant en charge les macros
spanish.TypeMacroWorkbook=Libro habilitado para macros
german.TypeMacroWorkbook=Arbeitsmappe mit Makros
italian.TypeMacroWorkbook=Cartella di lavoro con attivazione macro
english.TypeTemplate=Template
french.TypeTemplate=Modèle
spanish.TypeTemplate=Plantilla
german.TypeTemplate=Vorlage
italian.TypeTemplate=Modello
english.TypeMacroTemplate=Macro-Enabled Template
french.TypeMacroTemplate=Modèle prenant en charge les macros
spanish.TypeMacroTemplate=Plantilla habilitada para macros
german.TypeMacroTemplate=Vorlage mit Makros
italian.TypeMacroTemplate=Modello con attivazione macro
english.TypeCsv=CSV file
french.TypeCsv=Fichier CSV
spanish.TypeCsv=Archivo CSV
german.TypeCsv=CSV-Datei
italian.TypeCsv=File CSV
english.TypeTsv=Text file (tab-delimited)
french.TypeTsv=Fichier texte (séparateur : tabulation)
spanish.TypeTsv=Archivo de texto (delimitado por tabulaciones)
german.TypeTsv=Textdatei (Tabstopp-getrennt)
italian.TypeTsv=File di testo (delimitato da tabulazione)

[Tasks]
Name: "associate"; Description: "{cm:AssociateFiles}"; GroupDescription: "{cm:FileTypes}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\{#Target}\release\opensheet.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\OpenSheet"; Filename: "{app}\opensheet.exe"
Name: "{autodesktop}\OpenSheet"; Filename: "{app}\opensheet.exe"; Tasks: desktopicon

[Registry]
; OpenSheet in the "Open with" list of every type it reads.
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "OpenSheet"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\opensheet.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe\SupportedTypes"; ValueType: string; ValueName: ".xlsx"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe\SupportedTypes"; ValueType: string; ValueName: ".xlsm"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe\SupportedTypes"; ValueType: string; ValueName: ".xltx"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe\SupportedTypes"; ValueType: string; ValueName: ".xltm"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe\SupportedTypes"; ValueType: string; ValueName: ".csv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\opensheet.exe\SupportedTypes"; ValueType: string; ValueName: ".tsv"; ValueData: ""
; .xlsx
Root: HKA; Subkey: "Software\Classes\OpenSheet.xlsx"; ValueType: string; ValueName: ""; ValueData: "{cm:TypeWorkbook}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\OpenSheet.xlsx\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\opensheet.exe,0"
Root: HKA; Subkey: "Software\Classes\OpenSheet.xlsx\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\opensheet.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.xlsx\OpenWithProgids"; ValueType: string; ValueName: "OpenSheet.xlsx"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.xlsx"; ValueType: string; ValueName: ""; ValueData: "OpenSheet.xlsx"; Flags: uninsdeletevalue; Tasks: associate
; .xlsm
Root: HKA; Subkey: "Software\Classes\OpenSheet.xlsm"; ValueType: string; ValueName: ""; ValueData: "{cm:TypeMacroWorkbook}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\OpenSheet.xlsm\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\opensheet.exe,0"
Root: HKA; Subkey: "Software\Classes\OpenSheet.xlsm\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\opensheet.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.xlsm\OpenWithProgids"; ValueType: string; ValueName: "OpenSheet.xlsm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.xlsm"; ValueType: string; ValueName: ""; ValueData: "OpenSheet.xlsm"; Flags: uninsdeletevalue; Tasks: associate
; .xltx
Root: HKA; Subkey: "Software\Classes\OpenSheet.xltx"; ValueType: string; ValueName: ""; ValueData: "{cm:TypeTemplate}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\OpenSheet.xltx\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\opensheet.exe,0"
Root: HKA; Subkey: "Software\Classes\OpenSheet.xltx\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\opensheet.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.xltx\OpenWithProgids"; ValueType: string; ValueName: "OpenSheet.xltx"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.xltx"; ValueType: string; ValueName: ""; ValueData: "OpenSheet.xltx"; Flags: uninsdeletevalue; Tasks: associate
; .xltm
Root: HKA; Subkey: "Software\Classes\OpenSheet.xltm"; ValueType: string; ValueName: ""; ValueData: "{cm:TypeMacroTemplate}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\OpenSheet.xltm\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\opensheet.exe,0"
Root: HKA; Subkey: "Software\Classes\OpenSheet.xltm\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\opensheet.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.xltm\OpenWithProgids"; ValueType: string; ValueName: "OpenSheet.xltm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.xltm"; ValueType: string; ValueName: ""; ValueData: "OpenSheet.xltm"; Flags: uninsdeletevalue; Tasks: associate
; .csv
Root: HKA; Subkey: "Software\Classes\OpenSheet.csv"; ValueType: string; ValueName: ""; ValueData: "{cm:TypeCsv}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\OpenSheet.csv\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\opensheet.exe,0"
Root: HKA; Subkey: "Software\Classes\OpenSheet.csv\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\opensheet.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.csv\OpenWithProgids"; ValueType: string; ValueName: "OpenSheet.csv"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.csv"; ValueType: string; ValueName: ""; ValueData: "OpenSheet.csv"; Flags: uninsdeletevalue; Tasks: associate
; .tsv
Root: HKA; Subkey: "Software\Classes\OpenSheet.tsv"; ValueType: string; ValueName: ""; ValueData: "{cm:TypeTsv}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\OpenSheet.tsv\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\opensheet.exe,0"
Root: HKA; Subkey: "Software\Classes\OpenSheet.tsv\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\opensheet.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.tsv\OpenWithProgids"; ValueType: string; ValueName: "OpenSheet.tsv"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.tsv"; ValueType: string; ValueName: ""; ValueData: "OpenSheet.tsv"; Flags: uninsdeletevalue; Tasks: associate

[Run]
Filename: "{app}\opensheet.exe"; Description: "{cm:LaunchProgram,OpenSheet}"; Flags: nowait postinstall skipifsilent
