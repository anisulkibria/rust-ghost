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
    button('Preview',tools,async function(){var data=await request(path+'preview/');var modal=dialog('Publishing preview');var tabs=el('div',null,modal);tabs.style.cssText='display:flex;gap:8px;margin-bottom:16px';var frame=el('iframe',null,modal);frame.setAttribute('sandbox','allow-same-origin');frame.style.cssText='width:100%;height:60vh;border:1px solid var(--outline-variant,#ddd);border-radius:16px';frame.title='Publishing preview';function show(kind){if(kind==='web'){frame.removeAttribute('srcdoc');frame.src=data.preview.web_url;}else if(kind==='email'){frame.removeAttribute('src');frame.srcdoc=data.preview.email_html;}else{frame.removeAttribute('src');frame.srcdoc=data.preview.social_html;}}['web','email','social'].forEach(function(kind){button(kind[0].toUpperCase()+kind.slice(1),tabs,function(){show(kind);});});show('web');var email=input('Send a test newsletter to',modal,'','email');button('Send test newsletter',modal,async function(){if(!email.validity.valid||!email.value)throw new Error('Enter an email address.');await request(path+'test/','POST',{email:email.value});notice('Test newsletter sent');});});
    button('Newsletter links',tools,async function(){var data=await request('links/?post_id='+encodeURIComponent(id));var modal=dialog('Newsletter links');if(!data.links.length)el('p','Links appear after sending a newsletter with click tracking enabled.',modal);data.links.forEach(function(link){var row=el('div',null,modal);el('p',link.clicks+' clicks · '+link.unique_clicks+' members',row);var url=input('Destination',row,link.to,'url');button('Update destination',row,async function(){await request('links/'+link.id+'/','PUT',{links:[{to:url.value}]});notice('Destination updated');});});});
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
