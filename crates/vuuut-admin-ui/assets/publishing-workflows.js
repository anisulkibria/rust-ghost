/* Native admin controls for persistent publishing workflows. */
(function () {
  'use strict';
  var base = location.pathname.split('/ghost/')[0] + '/ghost/';
  var api = base + 'api/admin/';
  async function request(path, method, value) {
    var response = await fetch(api + path, {method: method || 'GET', credentials: 'same-origin', headers: {'Content-Type':'application/json'}, body:value === undefined ? undefined : JSON.stringify(value)});
    var data = response.status === 204 ? {} : await response.json();
    if (!response.ok) throw new Error(data.errors && data.errors[0] && data.errors[0].message || 'The operation failed.');
    return data;
  }
  function el(tag, text, parent) { var node = document.createElement(tag); if (text) node.textContent = text; if (parent) parent.appendChild(node); return node; }
  // The editor's actions live in a card in the settings column
  // (`#editor-actions`, drawn by the server); outside it this falls back to a
  // row of chips under the form, as before.
  var ACTION_ICONS={'Preview':'M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5c-1.73-4.39-6-7.5-11-7.5zM12 17a5 5 0 1 1 0-10 5 5 0 0 1 0 10zm0-8a3 3 0 1 0 0 6 3 3 0 0 0 0-6z','Duplicate':'M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z','Revision history':'M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.95 8.95 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z','Gift link':'M20 7h-2.2A3 3 0 0 0 12 4.4 3 3 0 0 0 6.2 7H4a1 1 0 0 0-1 1v3h18V8a1 1 0 0 0-1-1zM9 7a1 1 0 1 1 1-1v1H9zm6 0h-1V6a1 1 0 1 1 1 1zM4 13v7a1 1 0 0 0 1 1h6v-8H4zm9 8h6a1 1 0 0 0 1-1v-7h-7v8z','Newsletter links':'M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7a5 5 0 0 0 0 10h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4a5 5 0 0 0 0-10z'};
  function actionsHost(){var host=document.getElementById('editor-actions');if(host)return host;if(document.getElementById('editor-head-actions'))return {id:'editor-actions',appendChild:function(){}};var row=el('div',null,document.getElementById('editor-form').parentNode);row.style.cssText='display:flex;gap:8px;flex-wrap:wrap;margin:16px 0';return row;}
  function button(text, parent, fn) {
    // Preview is an everyday action: it goes in the editor's top bar.
    var head=document.getElementById('editor-head-actions');
    if(head && text==='Preview' && parent && parent.id==='editor-actions'){
      var top=document.createElement('button');top.type='button';top.className='btn text';
      top.innerHTML='<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="'+ACTION_ICONS.Preview+'"/></svg>';
      var lbl=document.createElement('span');lbl.className='lbl';lbl.textContent=' Preview';top.appendChild(lbl);top.title='Preview';
      head.appendChild(top);
      top.addEventListener('click',async function(){top.disabled=true;try{await fn();}catch(error){notice(error.message);}finally{top.disabled=false;}});
      return top;
    }
    if(parent && parent.id==='editor-actions'){
      var row=document.createElement('button');row.type='button';row.className='ed-action';
      row.innerHTML='<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="'+(ACTION_ICONS[text]||ACTION_ICONS.Preview)+'"/></svg>';
      el('span',text,row);
      // Preview first, the rest after what the server drew
      parent.appendChild(row);
      row.addEventListener('click',async function(){row.disabled=true;try{await fn();}catch(error){notice(error.message);}finally{row.disabled=false;}});
      return row;
    }
    var node=el('button', text, parent);node.type='button';node.className='btn tonal small';node.addEventListener('click',async function(){node.disabled=true;try{await fn();}catch(error){notice(error.message);}finally{node.disabled=false;}});return node;}
  function notice(text) { var node=el('div',text,document.body);node.setAttribute('role','status');node.style.cssText='position:fixed;bottom:24px;left:50%;transform:translateX(-50%);padding:14px 24px;border-radius:18px;background:var(--inverse-surface,#303034);color:var(--inverse-on-surface,#fff);z-index:100000;max-width:90vw;box-shadow:0 6px 24px #0003';setTimeout(function(){node.remove();},5000); }
  function dialog(title) {var modal=el('dialog',null,document.body);modal.style.cssText='width:min(860px,92vw);max-height:90vh;border:0;border-radius:28px;padding:28px;background:var(--surface,#fff);color:var(--on-surface,#222);box-shadow:0 12px 80px #0005';var head=el('div',null,modal);head.style.cssText='display:flex;justify-content:space-between;align-items:center;gap:24px;margin-bottom:20px';el('h2',title,head);button('Close',head,function(){modal.close();});modal.addEventListener('close',function(){modal.remove();});modal.showModal();return modal;}
  function input(label,parent,value,type) {var wrap=el('label',null,parent);wrap.className='field fixed';wrap.style.display='block';var node=el('input',null,wrap);node.type=type||'text';node.value=value||'';node.placeholder=' ';el('span',label,wrap);return node;}
  function outputLink(title,url) {var modal=dialog(title);var field=input('Link',modal,url);field.readOnly=true;button('Copy link',modal,async function(){await navigator.clipboard.writeText(url);notice('Link copied');});}


  var recovery=document.getElementById('password-recovery');
  if(recovery){var token=new URLSearchParams(location.search).get('token');var field=input(token?'New password':'Staff email',document.getElementById('recovery-fields'),'',token?'password':'email');field.required=true;if(token)field.minLength=10;recovery.onsubmit=async function(event){event.preventDefault();var submit=recovery.querySelector('button[type="submit"]');submit.disabled=true;try{var data=await request('authentication/password_reset/',token?'PUT':'POST',{password_reset:[token?{token:token,newPassword:field.value}:{email:field.value}]});document.getElementById('recovery-message').textContent=token?'Your password was reset. You can now sign in.':data.password_reset[0].message;field.value='';}catch(error){document.getElementById('recovery-message').textContent=error.message;}finally{submit.disabled=false;}};}

  // Tags panel: find or add. Typing filters the chips (ticked ones always
  // stay visible); Enter or Add ticks an exact match, or creates the tag
  // through the Admin API and adds it ticked. Ticking fires `change`, which
  // the editor treats like any other setting.
  (function(){
    var input=document.getElementById('tag-new');
    if(!input)return;
    var add=document.getElementById('tag-new-btn'),msg=document.getElementById('tag-new-msg'),chips=document.getElementById('tag-chips'),empty=document.getElementById('tag-empty');
    var CHECK='<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/></svg>';
    function all(){return Array.prototype.slice.call(chips.querySelectorAll('.fchip'));}
    function name(chip){return chip.textContent.trim();}
    function exact(q){q=q.trim().toLowerCase();return all().find(function(c){return name(c).toLowerCase()===q;});}
    function say(text,bad){msg.hidden=!text;msg.textContent=text||'';msg.classList.toggle('bad',!!bad);}
    function refresh(){
      var q=input.value.trim().toLowerCase();
      all().forEach(function(c){var box=c.querySelector('input');c.hidden=!!q&&!box.checked&&name(c).toLowerCase().indexOf(q)<0;});
      var match=exact(input.value);
      add.disabled=!q;
      add.lastChild.textContent=match?(match.querySelector('input').checked?' Added':' Select'):' Add';
      if(match&&match.querySelector('input').checked)add.disabled=true;
    }
    function tick(chip){var box=chip.querySelector('input');if(!box.checked){box.checked=true;box.dispatchEvent(new Event('change',{bubbles:true}));}}
    async function commit(){
      var value=input.value.trim();
      if(!value)return;
      var match=exact(value);
      if(match){tick(match);input.value='';refresh();say('');return;}
      add.disabled=true;say('Creating “'+value+'”…');
      try{
        var data=await request('tags/','POST',{tags:[{name:value}]});
        var tag=data.tags[0];
        var chip=document.createElement('label');chip.className='fchip';
        var box=document.createElement('input');box.type='checkbox';box.name='tags';box.value=tag.id;
        var ck=document.createElement('span');ck.className='ck';ck.innerHTML=CHECK;
        chip.appendChild(box);chip.appendChild(ck);chip.appendChild(document.createTextNode(tag.name));
        chips.insertBefore(chip,chips.firstChild);
        if(empty)empty.remove();
        tick(chip);
        input.value='';refresh();say('Tag “'+tag.name+'” created and added.');
        setTimeout(function(){say('');},3000);
      }catch(error){say(error.message,true);refresh();}
    }
    input.addEventListener('input',function(){refresh();say('');});
    input.addEventListener('keydown',function(e){if(e.key==='Enter'){e.preventDefault();commit();}if(e.key==='Escape'){input.value='';refresh();}});
    add.addEventListener('click',commit);
    refresh();
  })();

  var editor=document.getElementById('editor-form');
  var match=location.pathname.match(/\/(posts|pages)\/([^/]+)\/$/);
  // The newsletter audience panel used to be assembled here, from bare
  // browser widgets appended after the rest of the rail. It is rendered by
  // the server now, with the same helpers as every other panel, so that it
  // looks like one of them — see `audience_panel` in lib.rs. The
  // `editor/newsletters/` and `posts/{id}/audience/` endpoints it called are
  // left in place; they are part of the Admin API, not just this script.

  if(editor && match && match[2]!=='new') {
    var resource=match[1],id=match[2],path=resource+'/'+encodeURIComponent(id)+'/';
    var tools=actionsHost();
    button('Duplicate',tools,async function(){var data=await request(path+'copy/','POST',{});location.href=base+resource+'/'+data[resource][0].id+'/';});
    button('Revision history',tools,async function(){var data=await request(path+'revisions/');var modal=dialog('Revision history');if(!data.revisions.length)el('p','Previous versions appear here after you save changes.',modal);data.revisions.forEach(function(revision){var row=el('div',null,modal);row.style.cssText='border-top:1px solid var(--outline-variant,#ddd);padding:16px 0';el('strong',revision.title||'Untitled',row);el('p',new Date(revision.created_at.replace(' ','T')+'Z').toLocaleString(),row);button('Preview version',row,function(){var preview=dialog(revision.title||'Previous version');var frame=el('iframe',null,preview);frame.setAttribute('sandbox','');frame.title='Saved post content';frame.style.cssText='width:100%;height:50vh;border:0';frame.srcdoc=revision.post.html||'<p>No body content</p>';});button('Restore this version',row,async function(){await request(path+'revisions/'+revision.id+'/','PUT',{});location.reload();});});});
    button('Gift link',tools,async function(){var modal=dialog('Share this article');el('p','Create a link that lets anyone with it read this article. Creating another link revokes the previous one.',modal);button('Create or reset link',modal,async function(){var data=await request(path+'gift/','POST',{});outputLink('Article gift link',data.gift_links[0].url);});button('Revoke link',modal,async function(){await request(path+'gift/','DELETE');notice('Gift link revoked');modal.close();});});
    button('Newsletter links',tools,async function(){var data=await request('links/?post_id='+encodeURIComponent(id));var modal=dialog('Newsletter links');if(!data.links.length)el('p','Links appear after sending a newsletter with click tracking enabled.',modal);data.links.forEach(function(link){var row=el('div',null,modal);el('p',link.clicks+' clicks · '+link.unique_clicks+' members',row);var url=input('Destination',row,link.to,'url');button('Update destination',row,async function(){await request('links/'+link.id+'/','PUT',{links:[{to:url.value}]});notice('Destination updated');});});});
  }

  if(editor && match) {
    var previewTools=actionsHost();
    function snapshotFields() {
      if(editor.ghostPreviewData) return editor.ghostPreviewData();
      var fields=new URLSearchParams();
      new FormData(editor).forEach(function(value,key){if(typeof value==='string')fields.append(key,value);});
      return fields;
    }
    async function previewRequest(fields,test) {
      var endpoint=editor.getAttribute('action').replace(/\/?$/,'/')+'preview/'+(test?'test/':'');
      var response=await fetch(endpoint,{method:'POST',credentials:'same-origin',headers:{Accept:'application/json'},body:fields});
      if(response.status===204) return {};
      var result=await response.json();
      if(!response.ok) throw new Error(result.message || result.errors?.[0]?.message || 'Preview failed.');
      return result;
    }
    button('Preview',previewTools,async function(){
      var fields=snapshotFields();
      var data=(await previewRequest(fields,false)).preview;
      var modal=dialog('Publishing preview');
      // Its own class for the grid rows, plus the box metrics restated
      // inline: `dialog()` writes width/max-height/padding as inline styles,
      // which beat any stylesheet, and the shared `dialog` rule separately
      // clamps max-width to 420px — narrower than a phone. Left alone, every
      // device rendered at mobile width because the frame inherited that
      // clamp. Overriding only `width` is not enough; `max-width` is the one
      // that was actually biting.
      modal.className='preview';
      modal.style.width='min(1600px, calc(100vw - 24px))';
      modal.style.maxWidth='none';
      modal.style.height='calc(100dvh - 24px)';
      modal.style.maxHeight='none';
      modal.style.padding='12px 14px';
      // Everything that is not the page itself shares the heading row: what
      // to preview, which device, and Close. A separate toolbar row cost
      // ~46px of height that the page could have had, and the page is the
      // only reason the dialog is open.
      var head=modal.firstChild;
      head.style.gap='16px';
      var closeButton=head.lastChild;
      var tabs=el('div',null,head);tabs.style.cssText='display:flex;gap:6px;flex-wrap:wrap';
      var devices=el('div',null,head);devices.style.cssText='display:flex;gap:6px;flex-wrap:wrap;margin-left:auto';
      // `el` appends, so both landed after Close; put Close back on the end.
      head.appendChild(closeButton);
      var viewport=el('div',null,modal);
      viewport.style.cssText='overflow:auto;background:var(--surface-container,#eee);padding:8px;box-sizing:border-box;border-radius:12px';
      // The frame is sized in the device's own CSS pixels and then scaled to
      // fit the modal, rather than being stretched to whatever width the
      // modal happens to have. That distinction is the whole feature: the
      // page inside sizes itself to the frame's width, so a frame that is
      // merely "as wide as the dialog" shows the layout for that width and
      // not the one the publisher asked to see. Desktop is a real 1320px
      // viewport even on a laptop screen.
      var stage=el('div',null,viewport);
      stage.style.cssText='margin:0 auto;transform-origin:top center';
      var frame=el('iframe',null,stage);frame.setAttribute('sandbox','allow-same-origin');
      frame.style.cssText='display:block;border:1px solid var(--outline-variant,#ddd);border-radius:12px;background:white';
      frame.title='Publishing preview';

      var DEVICES=[['Desktop',1320],['Tablet',768],['Mobile',375]];
      var deviceWidth=DEVICES[0][1];
      function fit(){
        // Available width inside the viewport's padding.
        var available=Math.max(320,viewport.clientWidth-16);
        var scale=Math.min(1,available/deviceWidth);
        var height=Math.max(320,viewport.clientHeight-16);
        frame.style.width=deviceWidth+'px';
        // Undo the scale for height too, so the frame fills the stage after
        // scaling instead of leaving a gap below it.
        frame.style.height=Math.round(height/scale)+'px';
        stage.style.transform=scale<1?'scale('+scale+')':'none';
        // The scaled box still occupies its unscaled size in layout, so the
        // stage is given the post-scale width to stay centred and to stop the
        // viewport scrolling sideways for no reason.
        stage.style.width=deviceWidth+'px';
        viewport.style.setProperty('--preview-scale',String(scale));
      }
      var deviceButtons=[];
      DEVICES.forEach(function(device){
        var control=button(device[0],devices,function(){
          deviceWidth=device[1];
          fit();
          deviceButtons.forEach(function(item){item.setAttribute('aria-pressed',String(item===control));});
        });
        control.setAttribute('aria-pressed',String(device[0]==='Desktop'));deviceButtons.push(control);
      });
      // Resizing the window changes how much the frame has to be scaled by.
      var onResize=function(){fit();};
      window.addEventListener('resize',onResize);
      modal.addEventListener('close',function(){window.removeEventListener('resize',onResize);});
      // The social tab shows two cards side by side, the way Ghost does:
      // Facebook and X read different meta fields and crop and truncate
      // differently, so one generic card cannot tell a publisher whether
      // either of them is right.
      var socialPane=el('div',null,viewport);
      socialPane.style.cssText='display:none;gap:16px;flex-wrap:wrap;justify-content:center';
      var socialFrames=['social','twitter'].map(function(kind){
        var card=el('iframe',null,socialPane);
        card.setAttribute('sandbox','');
        card.title=kind==='social'?'Facebook card preview':'X card preview';
        card.style.cssText='flex:1 1 320px;min-width:300px;max-width:420px;height:420px;border:0;background:white;border-radius:16px';
        return {kind:kind,node:card};
      });

      var tabButtons=[];
      function show(kind){
        var social=kind==='social';
        socialPane.style.display=social?'flex':'none';
        stage.style.display=social?'none':'block';
        if(social){
          socialFrames.forEach(function(item){item.node.srcdoc=data[item.kind+'_html']||'';});
          message.textContent='';
        } else if(kind==='email' && data.email_error){
          frame.srcdoc='';message.textContent=data.email_error;
        } else {
          frame.srcdoc=data[kind+'_html']||'';
          message.textContent=(kind==='email'&&data.newsletter_name)?('Rendered as “'+data.newsletter_name+'”'):'';
        }
        // The device sizes are about the web and email views; a social card
        // is a fixed size on every device.
        devices.style.display=social?'none':'flex';
        if(!social) fit();
        tabButtons.forEach(function(item){item.node.setAttribute('aria-pressed',String(item.kind===kind));});
      }
      [['web','Browser'],['email','Email'],['social','Social']].forEach(function(entry){
        var node=button(entry[1],tabs,function(){show(entry[0]);});
        node.setAttribute('aria-pressed',String(entry[0]==='web'));
        tabButtons.push({node:node,kind:entry[0]});
      });
      // Everything below the frame on one line. These are occasional
      // actions — copy a link, send yourself a test — and stacking them as
      // three labelled rows cost more height than the preview could spare.
      var footer=el('div',null,modal);
      footer.style.cssText='display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:8px';
      // Fields in here are single-line controls, not a form to fill in.
      var compact=function(field,basis){
        field.parentNode.style.cssText='position:relative;display:block;margin:0;flex:1 1 '+basis;
        field.style.cssText='height:36px;padding:0 12px;box-sizing:border-box;width:100%';
        // The floating label has nothing to float over at this height.
        var span=field.parentNode.querySelector('span');
        if(span) span.remove();
        return field;
      };

      var message=el('p',null,footer);
      message.setAttribute('role','status');
      message.style.cssText='margin:0;flex:0 1 auto;font-size:13px;color:var(--on-surface-variant,#666)';

      // The shareable draft link. Only for a saved post: the link serves what
      // is stored, so offering it for unsaved edits would hand somebody a URL
      // that does not show what is on screen.
      if(data.web_url && match[2]!=='new'){
        var shareField=compact(input('',footer),'220px');
        shareField.value=data.web_url;shareField.readOnly=true;
        shareField.title='Shareable link to the saved version';
        shareField.setAttribute('aria-label','Shareable link to the saved version');
        var copy=button('Copy link',footer,async function(){
          try{await navigator.clipboard.writeText(data.web_url);notice('Preview link copied');}
          catch(error){shareField.select();throw new Error('Copy the selected link with your keyboard.');}
        });
        copy.className='btn tonal small';
      }

      var email=compact(input('',footer,'','email'),'200px');
      email.placeholder='you@example.com';
      email.setAttribute('aria-label','Send this preview as a test newsletter to');
      var send=button('Send test',footer,async function(){
        if(!email.validity.valid||!email.value)throw new Error('Enter an email address.');
        var payload=new URLSearchParams(fields);payload.set('preview_email',email.value);
        await previewRequest(payload,true);notice('Test newsletter sent');
      });
      send.className='btn tonal small';

      show('web');
      // After the first layout pass, so clientWidth is the real one.
      fit();
    });
  }

  // Saved views. The control itself — the "Filter" menu in each list's
  // toolbar, its NQL field and its Apply button — is server-rendered and
  // works without script; this only fills in the part that needs the API:
  // the list of views this user has saved, and the button that saves one.
  // Anything injected here lands inside the rendered popover, so it inherits
  // the admin's own styles rather than arriving as bare browser widgets.
  var viewsSlot=document.querySelector('[data-views]');
  if(viewsSlot){
    var resource=viewsSlot.getAttribute('data-views');
    var nqlField=document.querySelector('[data-nql]');
    function go(filter){location.href=location.pathname+(filter?'?nql='+encodeURIComponent(filter):'');}
    function paint(views){
      viewsSlot.textContent='';
      if(!views.length){el('p','Saved views appear here.',viewsSlot).className='muted tiny';return;}
      views.forEach(function(view){
        var row=el('div',null,viewsSlot);row.className='view-row';
        var open=el('button',view.name,row);open.type='button';open.className='view-open';open.title=view.filter;
        open.onclick=function(){go(view.filter);};
        var drop=el('button',null,row);drop.type='button';drop.className='icon-btn danger tiny';
        drop.setAttribute('aria-label','Delete the view '+view.name);drop.innerHTML='&times;';
        drop.onclick=async function(){try{await request('views/'+view.id+'/','DELETE');row.remove();if(!viewsSlot.children.length)paint([]);}catch(error){notice(error.message);}};
      });
    }
    var saveName=document.querySelector('[data-view-name]');
    var saveBtn=document.querySelector('[data-view-save]');
    if(saveBtn)saveBtn.addEventListener('click',async function(){
      var name=(saveName&&saveName.value||'').trim();
      var filter=(nqlField&&nqlField.value||'').trim();
      if(!name||!filter){notice('Name the view and give it a filter first.');return;}
      saveBtn.disabled=true;
      try{await request('views/','POST',{resource:resource,name:name,filter:filter});saveName.value='';notice('View saved');paint((await request('views/?resource='+resource)).views);}
      catch(error){notice(error.message);}finally{saveBtn.disabled=false;}
    });
    request('views/?resource='+resource).then(function(data){paint(data.views);}).catch(function(){viewsSlot.textContent='';});
  }

  var member=location.pathname.match(/\/members\/([^/]+)\/$/);if(member&&!['import','new'].includes(member[1])){var heading=document.querySelector('h1');if(heading)button('View as member',heading.parentNode,async function(){var result=await request('members/'+member[1]+'/signin_urls/');outputLink('Single-use member sign-in link',result.member_signin_urls[0].url);});}

  // Theme-declared custom image fields keep their URL input and add uploads.
  if(/\/customizations\/$/.test(location.pathname)) {
    document.querySelectorAll('[data-theme-image]').forEach(function(control){var file=el('input',null,control.parentNode);file.type='file';file.accept='image/*';file.setAttribute('aria-label','Upload '+control.name);file.onchange=async function(){if(!file.files.length)return;var form=new FormData();form.append('file',file.files[0]);file.disabled=true;try{var response=await fetch(api+'images/upload/',{method:'POST',body:form,credentials:'same-origin'});var result=await response.json();if(!response.ok)throw new Error(result.errors[0].message);control.value=result.images[0].url;control.dispatchEvent(new Event('input',{bubbles:true}));notice('Image uploaded. Save design to apply it.');}catch(error){notice(error.message);}finally{file.disabled=false;}};});
    var form = document.getElementById('design-form');
    var panel = document.getElementById('design-preview');
    if (form && panel) {
      var frame = panel.querySelector('iframe');
      var status = panel.querySelector('[role="status"]');
      var endpoint = new URL('preview/', form.action);
      var baseTag = document.createElement('base');
      baseTag.href = new URL(panel.dataset.home, location.origin).href;
      var timer, pending, dirty = true;
      async function renderPreview() {
        if (!panel.open || !dirty) return;
        if (pending) pending.abort();
        var controller = new AbortController();
        pending = controller;
        if (!frame.hasAttribute('srcdoc')) {
          status.textContent = 'Loading preview…';
          status.hidden = false;
        }
        try {
          var values = Object.fromEntries(new FormData(form));
          var response = await fetch(endpoint, {
            method: 'POST',
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify(values),
            credentials: 'same-origin',
            signal: controller.signal
          });
          if (!response.ok) throw new Error('Preview request failed');
          var preview = await response.json();
          if (controller.signal.aborted) return;
          var head = /<head(?:\s[^>]*)?>/i;
          frame.srcdoc = head.test(preview.html)
            ? preview.html.replace(head, function (tag) { return tag + baseTag.outerHTML; })
            : preview.html.replace(/^(\s*<!doctype[^>]*>)?/i, function (tag) { return tag + baseTag.outerHTML; });
          status.hidden = true;
          dirty = false;
        } catch (error) {
          if (controller.signal.aborted) return;
          status.textContent = 'Could not load the homepage preview.';
          status.hidden = false;
        } finally {
          if (pending === controller) pending = null;
        }
      }
      panel.addEventListener('toggle', function () {
        if (panel.open) renderPreview();
        else if (pending) pending.abort();
      });
      form.addEventListener('input', function () {
        dirty = true;
        if (pending) pending.abort();
        clearTimeout(timer);
        timer = setTimeout(renderPreview, 500);
      });
      renderPreview();
    }
  }
})();
