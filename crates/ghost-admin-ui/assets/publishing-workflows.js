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
  function button(text, parent, fn) {var node=el('button', text, parent);node.type='button';node.className='btn tonal small';node.addEventListener('click',async function(){node.disabled=true;try{await fn();}catch(error){notice(error.message);}finally{node.disabled=false;}});return node;}
  function notice(text) { var node=el('div',text,document.body);node.setAttribute('role','status');node.style.cssText='position:fixed;bottom:24px;left:50%;transform:translateX(-50%);padding:14px 24px;border-radius:18px;background:var(--inverse-surface,#303034);color:var(--inverse-on-surface,#fff);z-index:100000;max-width:90vw;box-shadow:0 6px 24px #0003';setTimeout(function(){node.remove();},5000); }
  function dialog(title) {var modal=el('dialog',null,document.body);modal.style.cssText='width:min(860px,92vw);max-height:90vh;border:0;border-radius:28px;padding:28px;background:var(--surface,#fff);color:var(--on-surface,#222);box-shadow:0 12px 80px #0005';var head=el('div',null,modal);head.style.cssText='display:flex;justify-content:space-between;align-items:center;gap:24px;margin-bottom:20px';el('h2',title,head);button('Close',head,function(){modal.close();});modal.addEventListener('close',function(){modal.remove();});modal.showModal();return modal;}
  function input(label,parent,value,type) {var wrap=el('label',null,parent);wrap.className='field fixed';wrap.style.display='block';var node=el('input',null,wrap);node.type=type||'text';node.value=value||'';node.placeholder=' ';el('span',label,wrap);return node;}
  function outputLink(title,url) {var modal=dialog(title);var field=input('Link',modal,url);field.readOnly=true;button('Copy link',modal,async function(){await navigator.clipboard.writeText(url);notice('Link copied');});}


  var recovery=document.getElementById('password-recovery');
  if(recovery){var token=new URLSearchParams(location.search).get('token');var field=input(token?'New password':'Staff email',document.getElementById('recovery-fields'),'',token?'password':'email');field.required=true;if(token)field.minLength=10;recovery.onsubmit=async function(event){event.preventDefault();var submit=recovery.querySelector('button[type="submit"]');submit.disabled=true;try{var data=await request('authentication/password_reset/',token?'PUT':'POST',{password_reset:[token?{token:token,newPassword:field.value}:{email:field.value}]});document.getElementById('recovery-message').textContent=token?'Your password was reset. You can now sign in.':data.password_reset[0].message;field.value='';}catch(error){document.getElementById('recovery-message').textContent=error.message;}finally{submit.disabled=false;}};}

  var editor=document.getElementById('editor-form');
  var match=location.pathname.match(/\/(posts|pages)\/([^/]+)\/$/);
  if(editor&&match&&match[1]==='posts') {
    var audienceSide=editor.querySelector('.editor-side');
    if(audienceSide){var details=el('details',null,audienceSide);details.className='card panel';el('summary','Newsletter audience',details);var fields=el('fieldset',null,details);fields.disabled=true;fields.style.cssText='border:0;padding:16px';var marker=el('input',null,fields);marker.type='hidden';marker.name='_audience';marker.value='1';var label=el('label',null,fields);label.textContent='Newsletter';var newsletter=el('select',null,label);newsletter.name='newsletter_id';newsletter.style.cssText='display:block;width:100%;margin:8px 0 16px';var defaultOption=el('option','Default newsletter',newsletter);defaultOption.value='';var filter=input('Audience filter · optional NQL',fields);filter.name='audience_filter';var previewLabel=el('label',null,fields);previewLabel.style.display='block';var includePreview=el('input',null,previewLabel);includePreview.type='checkbox';includePreview.name='newsletter_include_preview';includePreview.value='1';el('span',' Send a paywalled preview to subscribers without access',previewLabel);el('p','Leave the filter empty to use all eligible subscribers. Example: labels:vip+status:paid. Subscribers outside the selected newsletter are excluded.',fields).className='muted';
      Promise.all([request('editor/newsletters/'),match[2]==='new'?Promise.resolve({audience:{}}):request('posts/'+match[2]+'/audience/')]).then(function(values){values[0].newsletters.forEach(function(item){var option=el('option',item.name,newsletter);option.value=item.id;});var saved=values[1].audience;newsletter.value=saved.newsletter_id||'';filter.value=saved.filter||'';includePreview.checked=!!saved.include_preview;fields.disabled=false;}).catch(function(error){el('p',error.message,details);});
    }
  }

  if(editor && match && match[2]!=='new') {
    var resource=match[1],id=match[2],path=resource+'/'+encodeURIComponent(id)+'/';
    var tools=el('div',null,editor.parentNode);tools.style.cssText='display:flex;gap:8px;flex-wrap:wrap;margin:16px 0';
    button('Duplicate',tools,async function(){var data=await request(path+'copy/','POST',{});location.href=base+resource+'/'+data[resource][0].id+'/';});
    button('Revision history',tools,async function(){var data=await request(path+'revisions/');var modal=dialog('Revision history');if(!data.revisions.length)el('p','Previous versions appear here after you save changes.',modal);data.revisions.forEach(function(revision){var row=el('div',null,modal);row.style.cssText='border-top:1px solid var(--outline-variant,#ddd);padding:16px 0';el('strong',revision.title||'Untitled',row);el('p',new Date(revision.created_at.replace(' ','T')+'Z').toLocaleString(),row);button('Preview version',row,function(){var preview=dialog(revision.title||'Previous version');var frame=el('iframe',null,preview);frame.setAttribute('sandbox','');frame.title='Saved post content';frame.style.cssText='width:100%;height:50vh;border:0';frame.srcdoc=revision.post.html||'<p>No body content</p>';});button('Restore this version',row,async function(){await request(path+'revisions/'+revision.id+'/','PUT',{});location.reload();});});});
    button('Gift link',tools,async function(){var modal=dialog('Share this article');el('p','Create a link that lets anyone with it read this article. Creating another link revokes the previous one.',modal);button('Create or reset link',modal,async function(){var data=await request(path+'gift/','POST',{});outputLink('Article gift link',data.gift_links[0].url);});button('Revoke link',modal,async function(){await request(path+'gift/','DELETE');notice('Gift link revoked');modal.close();});});
    button('Newsletter links',tools,async function(){var data=await request('links/?post_id='+encodeURIComponent(id));var modal=dialog('Newsletter links');if(!data.links.length)el('p','Links appear after sending a newsletter with click tracking enabled.',modal);data.links.forEach(function(link){var row=el('div',null,modal);el('p',link.clicks+' clicks · '+link.unique_clicks+' members',row);var url=input('Destination',row,link.to,'url');button('Update destination',row,async function(){await request('links/'+link.id+'/','PUT',{links:[{to:url.value}]});notice('Destination updated');});});});
  }

  if(editor && match) {
    var previewTools=el('div',null,editor.parentNode);
    previewTools.style.cssText='display:flex;gap:8px;flex-wrap:wrap;margin:16px 0';
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
  if(/\/design\/$/.test(location.pathname)) {
    document.querySelectorAll('[data-theme-image]').forEach(function(control){var file=el('input',null,control.parentNode);file.type='file';file.accept='image/*';file.setAttribute('aria-label','Upload '+control.name);file.onchange=async function(){if(!file.files.length)return;var form=new FormData();form.append('file',file.files[0]);file.disabled=true;try{var response=await fetch(api+'images/upload/',{method:'POST',body:form,credentials:'same-origin'});var result=await response.json();if(!response.ok)throw new Error(result.errors[0].message);control.value=result.images[0].url;control.dispatchEvent(new Event('input',{bubbles:true}));notice('Image uploaded. Save design to apply it.');}catch(error){notice(error.message);}finally{file.disabled=false;}};});
    var form=document.querySelector('form[action$="/design/"]');if(form){var panel=el('details',null,form.parentNode);panel.className='card padded';el('summary','Live site preview',panel);var frame=el('iframe',null,panel);frame.title='Theme preview';frame.src=base.replace(/ghost\/$/,'');frame.style.cssText='width:100%;height:70vh;border:0;border-radius:16px;margin-top:16px';var timer;form.addEventListener('input',function(){clearTimeout(timer);timer=setTimeout(async function(){var values={};new FormData(form).forEach(function(value,key){if(typeof value==='string')values[key]=value;});try{var response=await fetch(base+'design/preview/',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(values),credentials:'same-origin'});if(!response.ok)return;frame.srcdoc=await response.text();}catch(error){/* Keep the current preview while the server is unreachable. */}},500);});}
  }
})();
