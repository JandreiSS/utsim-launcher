; Instalador do UTSim Launcher para Windows (NSIS 3).
;
; Instala por usuário, sem administrador, em %LOCALAPPDATA%\Programs\UTSim Launcher. O launcher
; instala os aplicativos da família em %LOCALAPPDATA%\UTSim\launcher (fora desta pasta).
;
; Atualização pelo próprio launcher: ele roda este instalador com "/S /reabrir". /S é o modo
; silencioso do NSIS (a mesma pasta, lida do registro); /reabrir abre o launcher novo no fim.
; Só a pasta do programa é sobrescrita: aplicativos, estado e código de acesso ficam em
; %LOCALAPPDATA%\UTSim\launcher e não são tocados.
; Compilar com scripts/instalador-launcher.sh, que chama:
;   makensis /DVERSAO=0.1.0 /DEXE=<utsim-launcher.exe> /DICONE=<launcher.ico> /DSAIDA=<instalador.exe> utsim-launcher.nsi

Unicode true

!ifndef VERSAO
  !define VERSAO "0.0.0"
!endif
!ifndef EXE
  !error "Defina /DEXE=<caminho do utsim-launcher.exe de release>"
!endif
!ifndef ICONE
  !error "Defina /DICONE=<caminho do launcher.ico>"
!endif
!ifndef SAIDA
  !define SAIDA "utsim-launcher-instalador.exe"
!endif

!define CHAVE_DESINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\UTSimLauncher"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

Name "UTSim Launcher ${VERSAO}"
OutFile "${SAIDA}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\UTSim Launcher"
InstallDirRegKey HKCU "Software\UTSim Launcher" "Pasta"
SetCompressor /SOLID lzma
BrandingText "UTSim Launcher ${VERSAO}"

!define MUI_ICON "${ICONE}"
!define MUI_UNICON "${ICONE}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "Este assistente instala o UTSim Launcher, que baixa, instala e atualiza os aplicativos da família UTSim.$\r$\n$\r$\nTenha à mão o código de acesso que você recebeu do autor.$\r$\n$\r$\nNão é preciso ser administrador."
!define MUI_FINISHPAGE_RUN "$INSTDIR\utsim-launcher.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir o UTSim Launcher agora"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "PortugueseBR"

VIProductVersion "${VERSAO}.0"
VIAddVersionKey /LANG=${LANG_PORTUGUESEBR} "ProductName" "UTSim Launcher"
VIAddVersionKey /LANG=${LANG_PORTUGUESEBR} "FileDescription" "Instalador do UTSim Launcher"
VIAddVersionKey /LANG=${LANG_PORTUGUESEBR} "FileVersion" "${VERSAO}"
VIAddVersionKey /LANG=${LANG_PORTUGUESEBR} "LegalCopyright" "Copyright (C) 2026 Jandrei"

Section "UTSim Launcher" SecPrincipal
  SectionIn RO
  SetOutPath "$INSTDIR"
  ; O launcher que pediu a atualização está fechando: espera o programa ficar livre (até 30 s).
  StrCpy $0 0
  ${Do}
    ClearErrors
    Delete "$INSTDIR\utsim-launcher.exe"
    ${IfNot} ${Errors}
      ${Break}
    ${EndIf}
    IntOp $0 $0 + 1
    ${If} $0 >= 120
      ${Break}
    ${EndIf}
    Sleep 250
  ${Loop}
  File "/oname=utsim-launcher.exe" "${EXE}"

  WriteUninstaller "$INSTDIR\desinstalar.exe"
  WriteRegStr HKCU "Software\UTSim Launcher" "Pasta" "$INSTDIR"
  WriteRegStr HKCU "${CHAVE_DESINST}" "DisplayName" "UTSim Launcher"
  WriteRegStr HKCU "${CHAVE_DESINST}" "DisplayVersion" "${VERSAO}"
  WriteRegStr HKCU "${CHAVE_DESINST}" "Publisher" "Projeto UTSim"
  WriteRegStr HKCU "${CHAVE_DESINST}" "DisplayIcon" "$INSTDIR\utsim-launcher.exe"
  WriteRegStr HKCU "${CHAVE_DESINST}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${CHAVE_DESINST}" "UninstallString" '"$INSTDIR\desinstalar.exe"'
  WriteRegStr HKCU "${CHAVE_DESINST}" "QuietUninstallString" '"$INSTDIR\desinstalar.exe" /S'
  WriteRegDWORD HKCU "${CHAVE_DESINST}" "NoModify" 1
  WriteRegDWORD HKCU "${CHAVE_DESINST}" "NoRepair" 1
SectionEnd

Section "Atalho no Menu Iniciar" SecAtalho
  SetShellVarContext current
  CreateDirectory "$SMPROGRAMS\UTSim Launcher"
  CreateShortcut "$SMPROGRAMS\UTSim Launcher\UTSim Launcher.lnk" "$INSTDIR\utsim-launcher.exe" "" "$INSTDIR\utsim-launcher.exe" 0
SectionEnd

Section /o "Atalho na área de trabalho" SecDesktop
  SetShellVarContext current
  CreateShortcut "$DESKTOP\UTSim Launcher.lnk" "$INSTDIR\utsim-launcher.exe" "" "$INSTDIR\utsim-launcher.exe" 0
SectionEnd

; Depois da atualização silenciosa pedida pelo launcher (/reabrir), abre o launcher novo.
Function .onInstSuccess
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/reabrir" $R1
  ${IfNot} ${Errors}
    Exec '"$INSTDIR\utsim-launcher.exe"'
  ${EndIf}
FunctionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecPrincipal} "O launcher (um único programa)."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAtalho} "Cria a pasta UTSim Launcher no Menu Iniciar. Os aplicativos instalados pelo launcher ganham atalhos na mesma pasta."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Cria um atalho do launcher na área de trabalho."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  SetShellVarContext current
  ; A pasta do Menu Iniciar também tem os atalhos dos aplicativos instalados pelo launcher.
  RMDir /r "$SMPROGRAMS\UTSim Launcher"
  Delete "$DESKTOP\UTSim Launcher.lnk"

  Delete "$INSTDIR\utsim-launcher.exe"
  Delete "$INSTDIR\desinstalar.exe"
  RMDir "$INSTDIR"

  ; Os aplicativos instalados pelo launcher ficam em outra pasta; pergunta antes de apagar.
  ${If} ${FileExists} "$LOCALAPPDATA\UTSim\launcher\*.*"
    MessageBox MB_YESNO|MB_ICONQUESTION "Remover também os aplicativos instalados pelo launcher (pasta $LOCALAPPDATA\UTSim\launcher)?$\r$\nSeus modelos e arquivos fora dessa pasta não são tocados." /SD IDNO IDNO manter
      RMDir /r "$LOCALAPPDATA\UTSim\launcher"
      RMDir "$LOCALAPPDATA\UTSim"
    manter:
  ${EndIf}

  DeleteRegKey HKCU "${CHAVE_DESINST}"
  DeleteRegKey HKCU "Software\UTSim Launcher"
SectionEnd
