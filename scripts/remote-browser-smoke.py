"""Real GUI/PTY host edits and browser, with an isolated SSH process fixture.
Authentication/trust are covered separately by test-remote-serial.py's real sshd.
"""
import json
import os
from pathlib import Path
import pty
import select
import shlex
import subprocess
import tempfile
import time
import tty
from gui_smoke_support import click_control, window_for

pairs=[pty.openpty() for _ in range(4)]
for master,slave in pairs: tty.setraw(slave)
paths=[os.ttyname(slave) for _,slave in pairs]
alias='/dev/serial/by-id/fixture'
with tempfile.TemporaryDirectory(prefix='signal-forge-remote-browser-') as directory:
    root=Path(directory);config_root=root/'config'/'signal-forge';config_root.mkdir(parents=True)
    bin_dir=root/'bin';bin_dir.mkdir()
    fake=bin_dir/'ssh'
    fake.write_text('''#!/usr/bin/env python3
import json,os,shlex,sys,time
with open(os.environ['REMOTE_CALLS'],'a') as output: output.write(json.dumps(sys.argv[1:])+'\\n')
parts=shlex.split(sys.argv[-1]);argument=parts[-1]
if argument=='discover-details':
    time.sleep(.15)
    if sys.argv[-2]=='unavailable': print('Could not resolve hostname unavailable',file=sys.stderr);sys.exit(255)
    if sys.argv[-2]=='empty': print('[]');sys.exit(0)
    print(os.environ['REMOTE_DEVICES']);sys.exit(0)
settings=json.loads(argument)
if sys.argv[-2]=='unavailable': print('Connection refused',file=sys.stderr);sys.exit(255)
settings['path']=json.loads(os.environ['REMOTE_ALIASES']).get(settings['path'],settings['path'])
os.execvp('python3',['python3','-c',parts[2],json.dumps(settings)])
''');fake.chmod(0o755)
    host=dict(name='Workshop',connection=dict(host='old-address',username=None,port=None))
    uris=['ssh://old-address'+path for path in paths[:2]]
    tabs=[dict(settings=dict(path=uri,baud=19200,data_bits=8,parity='None',stop_bits=1,flow='None'),remote_host='Workshop') for uri in uris]
    workspace=config_root/'workspace.json'
    workspace.write_text(json.dumps(dict(version=2,remote_hosts=[host,dict(name='Spare',connection=dict(host='empty',username=None,port=None))],ports=[tab['settings'] for tab in tabs]+[dict(path=paths[3],baud=19200,data_bits=8,parity='None',stop_bits=1,flow='None')],layout={'Split':dict(horizontal=True,fraction=.4,first={'Leaf':dict(tabs=[tabs[0]],active=0)},second={'Leaf':dict(tabs=[tabs[1]],active=0)})})))
    preset=dict(name='Fixed original target',payload='POLL',encoding='Text',escapes=True,ending='None',description='',shortcut='Ctrl+1',target={'Endpoint':'serial:'+uris[0]},repeat=None)
    preset_file=config_root/'presets.json';preset_file.write_text(json.dumps(dict(version=1,profiles=[dict(name='Remote',presets=[preset])])));preset_before=preset_file.read_bytes()
    calls=root/'calls.jsonl';log_path=root/'app.log'
    env=dict(os.environ,PATH=str(bin_dir)+os.pathsep+os.environ['PATH'],XDG_CONFIG_HOME=str(root/'config'),RUST_LOG='signal_forge=debug',REMOTE_CALLS=str(calls),REMOTE_DEVICES=json.dumps([dict(path=alias,target=paths[0]),dict(path=paths[1],target=paths[1])]),REMOTE_ALIASES=json.dumps({alias:paths[0]}))
    process=None;window=None
    def key(keys):
        subprocess.run(['xdotool','key','--window',window,'--clearmodifiers',keys],check=True);time.sleep(.2)
    def wait(predicate):
        deadline=time.monotonic()+8
        while not predicate():
            assert process.poll() is None,log_path.read_text()[-5000:]
            assert time.monotonic()<deadline,log_path.read_text()[-5000:]
            time.sleep(.05)
    def replace(control,text):
        click_control(window,log_path,control);key('ctrl+a');subprocess.run(['xdotool','type','--window',window,'--clearmodifiers',text],check=True);time.sleep(.2)
    def read(index,expected):
        data=b'';deadline=time.monotonic()+4
        while len(data)<len(expected) and time.monotonic()<deadline:
            if select.select([pairs[index][0]],[],[],.1)[0]:data+=os.read(pairs[index][0],4096)
        assert data==expected,(data,expected)
    def terminals(layout):
        return layout['Leaf']['tabs'] if 'Leaf' in layout else terminals(layout['Split']['first'])+terminals(layout['Split']['second'])
    with log_path.open('w') as log:
        try:
            process=subprocess.Popen(['target/debug/signal-forge','--no-update-check','--port',uris[0]],env=env,stderr=log);window=window_for(process)
            wait(lambda:'SSH remote serial connected' in log_path.read_text())
            click_control(window,log_path,'Remote host Workshop')
            click_control(window,log_path,'Open remote Workshop '+alias) # Focus existing tty, no duplicate alias terminal.
            click_control(window,log_path,'Open remote Workshop '+paths[1])
            wait(lambda:log_path.read_text().count('SSH remote serial connected')>=2)
            key('ctrl+s');assert len(terminals(json.loads(workspace.read_text())['layout']))==2
            click_control(window,log_path,'Refresh host Workshop')
            time.sleep(.4)
            # Local ports remain visible with live terminals and an expanded remote host.
            click_control(window,log_path,'Open local '+paths[3])
            key('Escape') # Cancel local setup without opening or sending.
            assert 'Opened '+paths[3] not in log_path.read_text(), 'Cancelled local setup opened a port'
            assert not select.select([pairs[3][0]],[],[],.1)[0], 'Cancelled local setup sent bytes'
            key('Escape');key('ctrl+1');read(0,b'POLL') # Refresh preserves live endpoint.
            click_control(window,log_path,'Edit host Workshop')
            replace('Remote address','bad address')
            click_control(window,log_path,'Apply remote edit');time.sleep(.2)
            assert json.loads(workspace.read_text())['remote_hosts'][0]['connection']['host']=='old-address'
            replace('Remote address','new-address')
            click_control(window,log_path,'Apply remote edit')
            saved=json.loads(workspace.read_text());saved_tabs=terminals(saved['layout'])
            assert saved['layout']['Split']['fraction']==.4
            assert [tab['endpoint_id'] for tab in saved_tabs]==['serial:'+uri for uri in uris]
            assert all(tab['settings']['path'].startswith('ssh://new-address/') for tab in saved_tabs)
            assert not select.select([master for master,_ in pairs],[],[],.2)[0], 'Editing sent bytes'
            # Re-open/focus from the browser after host change; original fixed preset still resolves.
            click_control(window,log_path,'Open remote Workshop '+alias)
            wait(lambda:log_path.read_text().count('SSH remote serial connected')>=3)
            key('Escape');key('ctrl+1');read(0,b'POLL')
            click_control(window,log_path,'Edit endpoint serial:'+uris[1])
            replace('Remote endpoint name','New sensor')
            replace('Remote device path',paths[2])
            click_control(window,log_path,'Baud options');click_control(window,log_path,'Baud 9600')
            click_control(window,log_path,'Apply remote edit')
            saved_tabs=terminals(json.loads(workspace.read_text())['layout'])
            assert saved_tabs[1]['settings']['path']=='ssh://new-address'+paths[2]
            assert saved_tabs[1]['endpoint_id']=='serial:'+uris[1]
            assert saved_tabs[1]['display_name']=='New sensor'
            assert saved_tabs[1]['settings']['baud']==9600
            click_control(window,log_path,'Remote host Spare');time.sleep(.4)
            subprocess.run(['import','-window',window,'/tmp/signal-forge-remote-browser.png'],check=True)
            key('ctrl+q');assert process.wait(timeout=5)==0
            assert preset_file.read_bytes()==preset_before
            process=subprocess.Popen(['target/debug/signal-forge','--no-update-check'],env=env,stderr=log);window=window_for(process);time.sleep(.3)
            assert not select.select([master for master,_ in pairs],[],[],.2)[0], 'Restart sent bytes'
            key('ctrl+s');restored=terminals(json.loads(workspace.read_text())['layout'])
            assert restored==saved_tabs
            # Failed reconnect keeps edited configuration; fix the host afterward.
            click_control(window,log_path,'Remote host Workshop');click_control(window,log_path,'Edit host Workshop')
            replace('Remote address','unavailable');click_control(window,log_path,'Apply remote edit')
            click_control(window,log_path,'Edit endpoint serial:'+uris[0]);key('Escape')
            key('ctrl+shift+o');time.sleep(.4)
            key('ctrl+s');assert all(tab['settings']['path'].startswith('ssh://unavailable/') for tab in terminals(json.loads(workspace.read_text())['layout']))
            assert preset_file.read_bytes()==preset_before
            key('ctrl+q');assert process.wait(timeout=5)==0
            invoked=[json.loads(line) for line in calls.read_text().splitlines()]
            assert any(args[-2]=='new-address' and shlex.split(args[-1])[-1]=='discover-details' for args in invoked)
            assert any(args[-2]=='unavailable' and shlex.split(args[-1])[-1]!='discover-details' for args in invoked)
            print('Visible local ports, async saved-host browsing/refresh, alias deduplication, shared host edits, stable IDs/preset targets, endpoint edits, persistence and failed-reconnect preservation passed.',flush=True)
        except BaseException:
            print(log_path.read_text()[-5000:],flush=True)
            if window and process.poll() is None:subprocess.run(['import','-window',window,'/tmp/signal-forge-remote-browser-failure.png'],check=False)
            raise
        finally:
            if process and process.poll() is None:process.terminate();process.wait(timeout=5)
            for master,slave in pairs:os.close(master);os.close(slave)
