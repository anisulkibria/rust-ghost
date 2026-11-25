#!/usr/bin/env python3
"""Build the demo content shipped with the bundled themes.

    python3 tools/theme-demo-content.py

writes, for each theme, content/themes/<theme>/demo/<theme>-demo.json: a
Ghost export (posts, pages, tags, authors) to load with Settings →
Advanced → Import content. Plinth also gets demo/plinth-products.csv, a
Shopify product CSV for Tools → E-commerce → Products → Import.

Pictures are picsum.photos (real photographs, stable per seed) and portraits
are randomuser.me; both are placeholders to replace with your own.
Re-running is deterministic: same ids, same text.
"""
import csv, hashlib, html, json, os, random

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'content', 'themes')
NOW = '2026-11-25T09:00:00.000Z'


def oid(*parts):
    return hashlib.sha1('|'.join(parts).encode()).hexdigest()[:24]


def uuid(*parts):
    h = hashlib.md5('|'.join(parts).encode()).hexdigest()
    return f'{h[:8]}-{h[8:12]}-4{h[13:16]}-a{h[17:20]}-{h[20:32]}'


def slugify(s):
    out = ''.join(c.lower() if c.isalnum() else '-' for c in s.replace('&', 'and').replace("'", '').replace('’', ''))
    while '--' in out:
        out = out.replace('--', '-')
    return out.strip('-')[:80]


def pic(seed, w=1600, h=1000):
    return f'https://picsum.photos/seed/{slugify(seed)}/{w}/{h}'


# ---------------------------------------------------------------- lexical
def _text(t, fmt=0):
    return {"detail": 0, "format": fmt, "mode": "normal", "style": "", "text": t, "type": "extended-text", "version": 1}


def _block(kind, children, **extra):
    b = {"children": children, "direction": "ltr", "format": "", "indent": 0, "type": kind, "version": 1}
    b.update(extra)
    return b


def body(blocks):
    """blocks: [("p", text) | ("h2", text) | ("quote", text) | ("ul", [items])] → (lexical, html, plaintext)."""
    nodes, parts, plain = [], [], []
    for kind, value in blocks:
        if kind == 'p':
            nodes.append(_block('paragraph', [_text(value)]))
            parts.append(f'<p>{html.escape(value)}</p>')
        elif kind in ('h2', 'h3'):
            nodes.append(_block('extended-heading', [_text(value)], tag=kind))
            parts.append(f'<{kind} id="{slugify(value)}">{html.escape(value)}</{kind}>')
        elif kind == 'quote':
            nodes.append(_block('extended-quote', [_text(value)]))
            parts.append(f'<blockquote>{html.escape(value)}</blockquote>')
        elif kind == 'ul':
            items = [_block('listitem', [_text(v)], value=i + 1) for i, v in enumerate(value)]
            nodes.append({"children": items, "direction": "ltr", "format": "", "indent": 0, "type": "list", "version": 1, "listType": "bullet", "start": 1, "tag": "ul"})
            parts.append('<ul>' + ''.join(f'<li>{html.escape(v)}</li>' for v in value) + '</ul>')
        plain.append(value if isinstance(value, str) else '\n'.join(value))
    lexical = {"root": {"children": nodes, "direction": "ltr", "format": "", "indent": 0, "type": "root", "version": 1}}
    return json.dumps(lexical), ''.join(parts), '\n\n'.join(plain)


# ---------------------------------------------------------------- export
class Export:
    def __init__(self, theme):
        self.theme = theme
        self.users, self.tags, self.posts, self.posts_tags, self.posts_authors, self.posts_meta = [], [], [], [], [], []
        self._tag_ids = {}
        self.n = 0
        self.settings, self.products = [], []

    def setting(self, key, value, group='site', kind='string'):
        self.settings.append({"id": oid(self.theme, 'setting', key), "group": group, "key": key,
                              "value": value if isinstance(value, str) else json.dumps(value), "type": kind,
                              "created_at": NOW, "updated_at": NOW})

    def menu(self, key, items):
        self.setting(key, [{"label": l, "url": u} for l, u in items], kind='array')

    def tier(self, name, monthly, yearly, description):
        self.products.append({"id": oid(self.theme, 'tier', name), "name": name, "slug": slugify(name), "active": 1,
                              "visibility": "public", "trial_days": 7, "description": description, "type": "paid",
                              "currency": "usd", "monthly_price": monthly, "yearly_price": yearly,
                              "created_at": NOW, "updated_at": NOW})

    def user(self, name, bio='', location='', image=None, website=None):
        uid = oid(self.theme, 'user', name)
        self.users.append({
            "id": uid, "name": name, "slug": slugify(name), "email": slugify(name).replace('-', '.') + '@example.com',
            "profile_image": image, "cover_image": None, "bio": bio or None, "website": website, "location": location or None,
            "status": "active", "visibility": "public", "created_at": NOW, "updated_at": NOW,
        })
        return uid

    def tag(self, name, description=None, image=None):
        internal = name.startswith('#')
        slug = 'hash-' + slugify(name[1:]) if internal else slugify(name)
        if slug in self._tag_ids:
            return self._tag_ids[slug]
        tid = oid(self.theme, 'tag', slug)
        self._tag_ids[slug] = tid
        self.tags.append({"id": tid, "name": name, "slug": slug, "description": description, "feature_image": image,
                          "visibility": "internal" if internal else "public", "created_at": NOW, "updated_at": NOW})
        return tid

    def post(self, title, blocks, tags=(), authors=(), excerpt=None, image=None, caption=None, featured=False,
             visibility='public', days_ago=0, kind='post', slug=None):
        self.n += 1
        pid = oid(self.theme, kind, title)
        lexical, html_, plain = body(blocks)
        day = 25 - days_ago
        month = 11
        while day < 1:
            day += 30
            month -= 1
        when = f'2026-{month:02d}-{day:02d}T{max(0, 22 - self.n % 20):02d}:{(59 - self.n * 7 % 60):02d}:00.000Z'
        self.posts.append({
            "id": pid, "uuid": uuid(self.theme, title), "title": title, "slug": slug or slugify(title), "type": kind,
            "lexical": lexical, "mobiledoc": None, "html": html_, "plaintext": plain,
            "feature_image": image, "featured": 1 if featured else 0, "status": "published", "visibility": visibility,
            "custom_excerpt": excerpt, "locale": None, "email_recipient_filter": "all",
            "reading_time": max(1, round(len(plain.split()) / 230)),
            "created_at": when, "updated_at": when, "published_at": when,
        })
        if caption:
            self.posts_meta.append({"id": oid(self.theme, 'meta', title), "post_id": pid, "feature_image_caption": caption})
        for i, t in enumerate(tags):
            self.posts_tags.append({"id": oid(self.theme, 'pt', title, t), "post_id": pid, "tag_id": self.tag(t), "sort_order": i})
        for i, a in enumerate(authors):
            self.posts_authors.append({"id": oid(self.theme, 'pa', title, a), "post_id": pid, "author_id": a, "sort_order": i})
        return pid

    def page(self, title, blocks, slug, excerpt=None, authors=()):
        return self.post(title, blocks, authors=authors, excerpt=excerpt, kind='page', slug=slug, days_ago=40)

    def write(self):
        out = {"db": [{"meta": {"exported_on": 1795000000000, "version": "6.0.0"}, "data": {
            "users": self.users, "tags": self.tags, "posts": self.posts, "posts_tags": self.posts_tags,
            "posts_authors": self.posts_authors, "posts_meta": self.posts_meta,
            "settings": self.settings, "products": self.products}}]}
        d = os.path.join(ROOT, self.theme, 'demo')
        os.makedirs(d, exist_ok=True)
        path = os.path.join(d, f'{self.theme}-demo.json')
        with open(path, 'w') as f:
            json.dump(out, f, ensure_ascii=False, indent=1)
        print(f'{path}: {len([p for p in self.posts if p["type"] == "post"])} posts, '
              f'{len([p for p in self.posts if p["type"] == "page"])} pages, {len(self.tags)} tags, {len(self.users)} authors')


def face(kind, n):
    return f'https://randomuser.me/api/portraits/{kind}/{n}.jpg'


# ================================================================ Broadsheet
def broadsheet(theme='broadsheet'):
    e = Export(theme)
    rng = random.Random(7)
    staff = {
        'jamal': e.user('Jamal Whitfield', 'Washington correspondent covering Congress and the courts.', 'Washington', face('men', 32)),
        'daniel': e.user('Daniel Park', 'Weather, climate and science reporter.', 'Chicago', face('men', 75)),
        'imogen': e.user('Imogen Hart', 'Business editor and host of the markets desk.', 'New York', face('women', 44)),
        'sofia': e.user('Sofia Almeida', 'Foreign and sports correspondent.', 'Lisbon', face('women', 68)),
        'marta': e.user('Marta Reyes', 'Managing editor.', 'New York', face('women', 21)),
    }
    for name, desc in [('U.S', 'National news.'), ('World', 'International news and diplomacy.'), ('Politics', 'Power, policy and the people who make it.'),
                       ('Business', 'Markets, companies and the economy.'), ('Technology', 'The industry and its consequences.'),
                       ('Sports', 'Leagues, labour and the games.'), ('Opinion', 'Columns, editorials and guest essays.'), ('Video', 'Explainers and documentaries.'),
                       ('Weather', 'Forecasts and the science behind them.'), ('Education', 'Schools, universities and learning.'),
                       ('Lifestyle', 'Food, travel and how we live.'), ('Health', 'Medicine, wellbeing and work.'), ('Arts', 'Culture and criticism.'),
                       ('Graphics', 'Data journalism.'), ('Sunday Essay', 'Long reads for a slower day.'), ('Wirecutter', 'What to buy, tested.')]:
        e.tag(name, desc)

    desk_lines = {
        'politics': ['Lawmakers from both parties described the moment as fragile but workable.', 'Aides said the timetable could still slip if amendments multiply on the floor.',
                     'The White House declined to comment on the details before a formal briefing.', 'Procedural votes this week will show whether the coalition holds.'],
        'world': ['Diplomats in both capitals described the agreement as overdue.', 'Officials said implementation would begin within ninety days.',
                  'Analysts expect neighbouring governments to respond cautiously.', 'The text was negotiated over eleven rounds across two years.'],
        'business': ['Traders said volumes were well above the thirty-day average.', 'Executives told analysts the outlook for the second half remains uneven.',
                     'The move was broadly anticipated, but its timing surprised some investors.', 'Regulators are expected to review the filing within weeks.'],
        'technology': ['Researchers cautioned that the results have not yet been independently replicated.', 'The companies involved say they welcome clearer standards.',
                       'Smaller developers worry about the cost of compliance.', 'Several pilots will run until the end of the year.'],
        'sports': ['Club officials described the talks as cordial and businesslike.', 'Players’ representatives want guarantees on revenue sharing.',
                   'Broadcasters are watching closely ahead of the next rights cycle.', 'The schedule for next season has not been finalised.'],
        'opinion': ['We should be honest about what this costs and who pays it.', 'There is a better argument here than either side is making.',
                    'The evidence points somewhere less comfortable than the slogans.', 'None of this is inevitable; all of it is a choice.'],
        'weather': ['Forecasters expect the pattern to persist into next week.', 'Emergency managers urged residents to check local alerts.',
                    'The models agreed unusually early on the storm’s track.', 'Temperatures will fall sharply behind the front.'],
        'education': ['District leaders said the data would guide spending next year.', 'Teachers’ unions asked for more time to review the plan.',
                      'Parents will be able to see school-level results online.', 'The pilot covered forty schools across three states.'],
        'lifestyle': ['The best tables are booked weeks ahead, but walk-ins are welcome at the bar.', 'Prices have crept up, though portions have not shrunk.',
                      'It is the kind of place that rewards a second visit.', 'Go early, and order whatever is on the board.'],
        'health': ['Clinicians said the findings match what they see on the ward.', 'The study followed participants for five years.',
                   'Employers are beginning to measure outcomes rather than enrolment.', 'Experts cautioned against reading too much into a single trial.'],
        'arts': ['The production runs through the spring.', 'Critics have been divided, audiences less so.', 'It is the company’s most ambitious season in a decade.', 'Tickets go on general sale next week.'],
    }

    def story(title, tags, author, excerpt, days, featured=False, visibility='public', video=False, caption=None, extra_author=None):
        desk = slugify(tags[0]).replace('u-s', 'politics').replace('sunday-essay', 'opinion').replace('graphics', 'business').replace('wirecutter', 'lifestyle')
        lines = desk_lines.get(desk, desk_lines['politics'])[:]
        rng.shuffle(lines)
        blocks = [('p', excerpt + ' ' + lines[0]), ('p', lines[1] + ' ' + lines[2]),
                  ('p', 'People familiar with the discussions, who spoke on condition of anonymity because they were not authorised to comment publicly, said the outcome had been in doubt until late in the process. ' + lines[3]),
                  ('h2', 'Why it matters'), ('p', 'The decision touches millions of people directly and many more indirectly. Supporters call it overdue; critics say it moves too fast and explains too little. Both camps agree the next six months will show whether it works.'),
                  ('h2', 'What happens next'), ('p', lines[3] + ' Our reporters will follow the story as it develops.'),
                  ('quote', 'It is the most consequential decision of the year, and it was made quietly.'),
                  ('p', 'Reporting was contributed by the ' + tags[0] + ' desk. This article has been updated with additional detail.')]
        t = list(tags) + (['#video', 'Video'] if video else [])
        authors = [staff[author]] + ([staff[extra_author]] if extra_author else [])
        return e.post(title, blocks, t, authors, excerpt, pic(title), caption, featured, visibility, days)

    S = story
    S('City Hall Live: Breaking Down Today’s Major Press Conference', ['Politics'], 'daniel', 'Our lead reporter brings you live, on-the-ground updates straight from the press room as officials unveil the latest policy changes and community initiatives.', 0, True, video=True)
    S('Markets in Motion: A Studio Breakdown of Today’s Stock Surges', ['Business'], 'imogen', 'Join our in-studio anchor desk for a comprehensive look at the day’s market trends, focusing on the unexpected shifts that drove today’s trading session.', 0, True, video=True)
    S('NFL Labor Talks Open Two Years Early as Both Sides Sense Window', ['Sports'], 'sofia', 'The current CBA does not expire until 2030, but both league and union leadership believe the conditions for a constructive negotiation will not be better.', 1, True, 'members', caption='Window Open: Both sides judge that the conditions for constructive negotiation are unusually favourable now.', extra_author='imogen')
    S('Northeast Braces for Late-Season Storm as Forecasters Warn of Record Cold', ['Weather'], 'jamal', 'A slow-moving system will bring heavy snow to inland areas and dangerous wind chills to the coast.', 1)
    S('The Science Behind Why Spring Storms Are Getting Stranger', ['Weather'], 'daniel', 'Warmer oceans are feeding storms more moisture, and the jet stream is behaving in unfamiliar ways.', 2, video=True)
    S('Senate Advances Bipartisan Border Bill After Months of Stalled Talks', ['U.S', 'Politics'], 'jamal', 'A narrow procedural vote clears the way for floor debate on a package that pairs new asylum authorities with funding.', 2, True)
    S('Letters: More Readers on Housing, This Time on the Production Side', ['Opinion', 'Politics'], 'sofia', 'Builders, planners and renters write in about what is really holding back new homes.', 3, True)
    S('The Quiet Re-Centralisation of American Political Power Deserves Attention', ['Opinion', 'Politics'], 'jamal', 'Decisions once made in statehouses are drifting back to Washington, with little debate.', 3, True, 'paid')
    S('Performance Pairing: Bill T. Jones Company With New Theatre Work in Brooklyn', ['Arts'], 'imogen', 'A choreographer and a playwright share a stage, and an argument, in a new production.', 4, True)
    S('Japan and South Korea Sign Long-Awaited Defence Cooperation Pact', ['World', 'Politics'], 'sofia', 'The agreement formalises information-sharing protocols and a joint operations framework that have been operating informally for nearly two years.', 4, True, caption='Formal Footing: The new pact codifies cooperation that has been operating informally between Japanese and South Korean defence services for nearly two years.')
    S('Brussels Greenlights Fifteenth Sanctions Package on Russia', ['World', 'Politics'], 'sofia', 'The new measures target a third-country sanctions-evasion network and tighten controls on dual-use goods.', 5, True)
    S('Gulf Coast Flood Season Opens Early as Atmospheric Rivers Shift North', ['Weather', 'World'], 'daniel', 'The Gulf Coast flood season has arrived six weeks early as shifting atmospheric patterns bring repeated heavy rain.', 5)
    S('EU Lawmakers Sign Off on AI Act Enforcement Mechanism', ['World', 'Technology', 'Politics'], 'imogen', 'Two years after the bloc’s landmark AI law took effect, member states have agreed how it will be policed.', 6)
    S('Graphics: How Global Trade Has Reorganised Across Three Years', ['Graphics', 'World'], 'imogen', 'Seven visualisations trace how global trade flows have reorganised over the past three years.', 6)
    S('Editorial: The Federal Statistical Agencies Need Modernisation, Not Cuts', ['Opinion', 'Politics'], 'marta', 'Good data is infrastructure. Starving it now will cost far more later.', 7)
    S('Federal Court Pauses White House Order on Federal Hiring Freeze', ['U.S', 'Politics'], 'jamal', 'A judge found the order likely exceeded the authority delegated by Congress.', 7)
    S('Charter Network Goes Public With Five-Year Reading Recovery Data', ['Education'], 'daniel', 'The network says its students closed most of the pandemic reading gap; outside researchers want to see the method.', 8)
    S('Sunday Essay: What Live Performance Still Does That Recordings Cannot', ['Opinion', 'Sunday Essay'], 'imogen', 'Something happens in a room full of strangers that no file can hold.', 8)
    S('White-Collar Hiring Freezes Quietly Spread Across Tech and Finance', ['Business'], 'imogen', 'Firms are not announcing layoffs; they are simply not replacing people who leave.', 9)
    S('Tech IPO Market Reopens With Three Major Listings in a Single Week', ['Business', 'Technology'], 'imogen', 'Bankers say the window is open, at least until the next bout of volatility.', 9)
    S('JPMorgan Spins Up Dedicated AI Trading Desk Inside Markets Division', ['Business', 'Technology'], 'imogen', 'The bank is moving a team of quants and engineers under one roof.', 10)
    S('Trophy Manhattan Office Trades at Premium Even as Class B Vacancies Rise', ['Business'], 'imogen', 'The best buildings are thriving; the rest are emptying.', 10)
    S('Graphics: How Tech Spending Has Diverged Across Sectors', ['Graphics', 'Business'], 'imogen', 'Software budgets are growing in some industries and shrinking in others.', 11)
    S('Pharma Industry Faces Largest Patent-Cliff Year on Record', ['Business', 'Health'], 'imogen', 'More than a dozen blockbuster drugs lose exclusivity within twelve months.', 11)
    S('Apprenticeship Programmes See Surge as Trades Pay Outpaces Graduate Salaries', ['Business', 'Education'], 'imogen', 'Applications are up by a third as starting wages in the trades climb.', 12)
    S('AI Agents Reach Enterprise-Production Status in Defined Workflows', ['Technology', 'Business'], 'imogen', 'Companies report that narrow, well-defined tasks are where the tools earn their keep.', 12)
    S('The Productivity Statistics Are Not Telling Us What We Think They Are', ['Opinion', 'Business'], 'marta', 'The numbers measure what is easy to count, not what matters.', 13)
    S('Boeing Closes Spirit AeroSystems Acquisition After Antitrust Concessions', ['Business'], 'imogen', 'The deal brings a key supplier back in-house after years of quality problems.', 13)
    S('Graphics: Where the Job Market Is Actually Tight, in Six Maps', ['Graphics', 'Business'], 'imogen', 'Shortages are concentrated in a handful of regions and occupations.', 14)
    S('Districts Quietly Roll Back the Most Aggressive AI Detection Tools', ['Education', 'Technology'], 'imogen', 'False-positive rates that affected real students have prompted districts to retire the screening tools they had deployed only eighteen months earlier.', 14)
    S('Frontier Model Evaluations Move Toward Standardised Disclosure', ['Technology'], 'daniel', 'An informal industry working group is converging on baseline disclosure requirements for frontier-model capability and safety evaluations.', 15)
    S('Guest Essay: I Lost My Job to AI Tooling. Here’s What I Learned', ['Opinion', 'Technology'], 'daniel', 'A senior content strategist whose role was eliminated reflects on what the broader public conversation gets right and wrong.', 15)
    S('NBA In-Season Tournament Reaches Format Maturity in Third Year', ['Sports'], 'sofia', 'The format adjustments after two years of operating data have produced a tournament players now take seriously.', 16)
    S('Premier League TV Rights Auction Sets Record on International Side', ['Sports', 'Business'], 'sofia', 'Overseas broadcasters paid more than ever, even as domestic bids flattened.', 16)
    S('Pro Cycling Team Budgets Reach New Records, But Sponsorship Strain Is Visible', ['Sports'], 'sofia', 'The richest teams are pulling away while smaller ones scramble for backers.', 17)
    S('The Real Cost of Sports Betting Is Showing Up in Places We Aren’t Watching', ['Opinion', 'Sports'], 'marta', 'Legalisation brought tax revenue. It also brought debt counsellors a new caseload.', 17)
    S('Sunday Essay: How We Lost the Long Arc of Public Attention', ['Opinion', 'Sunday Essay'], 'imogen', 'Our news cycle has become a series of short weather events.', 18)
    S('Sunday Essay: What the Bestseller List Has Always Been For', ['Arts', 'Sunday Essay'], 'imogen', 'It was never a measure of quality, and it was never meant to be.', 18)
    S('Sunday Essay: The Quiet Persistence of Things Made by Hand', ['Opinion', 'Sunday Essay'], 'jamal', 'In an age of infinite copies, the single object has gained a strange new authority.', 19)
    S('Sunday Essay: The Slow Renewal of Civic Practice in Mid-Sized Cities', ['Opinion', 'Sunday Essay'], 'jamal', 'Not in the capitals, but in places with one newspaper and two hospitals.', 19)
    S('Sunday Essay: What Sustained Reading Does to a Mind', ['Arts', 'Sunday Essay'], 'imogen', 'An hour with a book is not an hour away from the world.', 20)
    S('Sunday Essay: How the Opinion Section Has Adapted Across Formats', ['Opinion', 'Sunday Essay'], 'imogen', 'From the letters page to the podcast, the argument keeps changing shape.', 20)
    S('Sunday Essay: In Praise of the Quiet Things That Hold Lives Together', ['Opinion', 'Sunday Essay'], 'imogen', 'Routines, rituals and the people who keep them.', 21)
    S('Sunday Essay: The Quiet Work of Rebuilding Institutional Trust', ['Opinion', 'Sunday Essay'], 'jamal', 'Trust is not announced. It is earned in small, repeated acts.', 21)
    S('Education Department Issues Guidance on AI Use in Federal Aid Forms', ['Education'], 'imogen', 'Colleges may use the tools to process forms, but not to make final decisions.', 22)
    S('Court Rules in Favor of Universities on Race-Conscious Admissions Workaround', ['Education', 'U.S'], 'jamal', 'The ruling allows essays that discuss identity, within limits.', 22)
    S('Op-Doc Review: One Teacher, One Classroom, Across a Year', ['Opinion', 'Education'], 'imogen', 'An 18-minute documentary follows a single teacher across one academic year of a Detroit public elementary school.', 23, video=True)
    S('Where to Eat in London Right Now, From a Visitor Who Cooks', ['Lifestyle'], 'imogen', 'Five London restaurants worth planning a meal around, drawn from a visitor’s recent week of careful eating.', 23)
    S('What the Slow Return of Tailoring Is Really About', ['Lifestyle'], 'imogen', 'The contemporary tailoring revival is less about formality than about the kind of construction that survives daily wear.', 24)
    S('Workplace Wellness Programs Move From Perks to Operational Strategy', ['Health', 'Lifestyle'], 'imogen', 'After two decades as employee perks, wellness programs are being restructured around outcomes that the business can measure.', 24)
    S('The Quiet Power of Small Family Rituals That Don’t Need Names', ['Lifestyle'], 'sofia', 'The Sunday call, the shared recipe, the walk after dinner.', 25)
    S('Cooking the Spring Vegetables That Are Actually Worth Cooking', ['Lifestyle'], 'sofia', 'Skip the asparagus. Here are the ones that reward a little effort.', 25)
    S('Mental-Health Stigma at Work Has Receded, but the Practical Gaps Have Not', ['Health'], 'daniel', 'Employees talk more openly; the services have not kept up.', 26)
    S('Guest Essay: Why I Stopped Practising Emergency Medicine', ['Health', 'Opinion'], 'daniel', 'A physician explains why the job became impossible to do well.', 26)
    S('Sleep Research Confirms What Practitioners Have Been Saying for Years', ['Health'], 'daniel', 'Consistency matters more than duration, a large new study finds.', 27)
    S('Video Explainer: The Federal Reserve’s Balance Sheet, in Eight Minutes', ['Business'], 'marta', 'An eight-minute explainer video walks through the mechanics of the Fed’s balance sheet without oversimplifying.', 27, video=True)
    S('Op-Doc Review: A Housing Crisis Story Told Through One Building', ['Opinion'], 'imogen', 'A 14-minute documentary opinion piece examines a single Brooklyn apartment building across the past five years.', 28, video=True)
    S('Video: How Zoning Actually Works, and Why It Matters', ['Politics'], 'marta', 'A six-minute video maps the actual operational mechanisms through which zoning decisions affect housing supply.', 28, video=True)
    S('Opinion Audio: A Patient Audio Argument for Local Civic Engagement', ['Opinion'], 'jamal', 'A half-hour case for showing up at the meetings nobody attends.', 29)
    S('How Meteorologists Got the April Tornado Outbreak Wrong — and What They’re Changing', ['Weather'], 'daniel', 'The April tornado outbreak that moved through the central Plains and lower Mississippi Valley last month produced lessons forecasters are acting on.', 29)
    S('Games and Wirecutter: The Best Board Games to Buy in 2026', ['Wirecutter', 'Lifestyle'], 'daniel', 'Our recommendations across price points and player counts after testing thirty current board games.', 30)

    more = [
        ('Supreme Court Agrees to Hear Challenge to State Redistricting Maps', ['U.S', 'Politics'], 'jamal', 'The case could reshape how district lines are drawn before the next census.'),
        ('Rail Strike Averted After Overnight Deal on Crew Scheduling', ['U.S', 'Business'], 'jamal', 'Negotiators reached a tentative agreement hours before the deadline.'),
        ('Wildfire Smoke Prompts Air Alerts Across Three Midwestern States', ['U.S', 'Weather'], 'daniel', 'Officials advised sensitive groups to stay indoors through the weekend.'),
        ('Inside the Small-Town Race That Became a National Test', ['U.S', 'Politics'], 'jamal', 'A mayoral contest in a town of 9,000 has drawn money from across the country.'),
        ('Chip Makers Race to Secure Power for a New Generation of Fabs', ['Technology', 'Business'], 'imogen', 'Electricity, not land or labour, has become the scarcest input for new plants.'),
        ('The Open-Source Maintainers Holding Up the Internet Are Tired', ['Technology'], 'daniel', 'A survey of maintainers finds burnout, thin funding and growing security demands.'),
        ('Satellite Internet Reaches Rural Schools, With Strings Attached', ['Technology', 'Education'], 'daniel', 'Connectivity has arrived; so have contracts districts do not fully understand.'),
        ('Why Your Phone Battery Still Doesn’t Last, Explained', ['Technology'], 'daniel', 'Better chemistry keeps arriving. So do brighter screens and hungrier apps.'),
        ('Election Monitors Report Calm Vote in Coastal Provinces', ['World', 'Politics'], 'sofia', 'Turnout was high and incidents few, observers said.'),
        ('Port Strike Ripples Through European Supply Chains', ['World', 'Business'], 'sofia', 'Containers are piling up as dockworkers enter a second week of action.'),
        ('Drought Forces Hard Choices on the Mekong', ['World', 'Weather'], 'daniel', 'Farmers, fishers and hydropower operators are competing for a shrinking river.'),
        ('Coalition Talks Collapse as Junior Partner Walks Out', ['Politics', 'World'], 'sofia', 'A snap election now looks likely before the summer.'),
        ('Governors Push Back on Federal Broadband Rules', ['Politics', 'U.S'], 'jamal', 'Twenty states say the new requirements will delay construction.'),
        ('Women’s Football Attendance Records Fall Again', ['Sports'], 'sofia', 'Clubs are moving more fixtures to their main stadiums.'),
        ('Marathon Times Keep Falling. Shoes Are Only Part of the Story', ['Sports', 'Health'], 'sofia', 'Training science, pacing and deeper fields all play a role.'),
        ('A Tennis Prodigy, a Quiet Coach and a Very Loud Year', ['Sports'], 'sofia', 'At 17, she has already beaten three top-ten players.'),
        ('Teacher Shortages Ease in Cities, Worsen in Rural Districts', ['Education'], 'daniel', 'Pay rises helped urban schools; rural ones are still struggling to fill posts.'),
        ('Universities Rethink the Lecture After Attendance Drops', ['Education'], 'imogen', 'Smaller seminars and recorded talks are replacing the 300-seat hall.'),
        ('Mediterranean Diet Study Finds Benefits Start Within Months', ['Health'], 'daniel', 'Participants saw measurable changes in blood pressure within twelve weeks.'),
        ('A Museum Reopens With Its Storerooms on Display', ['Arts'], 'imogen', 'Visitors can now walk through the shelves where most of the collection lives.'),
        ('The Novel Everyone Is Arguing About This Autumn', ['Arts', 'Sunday Essay'], 'imogen', 'A slim book about a family dinner has divided critics and delighted readers.'),
        ('A Weekend in Porto Without a Single Plan', ['Lifestyle'], 'sofia', 'Walk downhill, eat what is on the board, and let the river decide.'),
    ]
    for i, (t, tg, a, x) in enumerate(more):
        story(t, tg, a, x, 31 + i)

    if theme == 'coverline':
        e.menu('navigation', [('Home', '/'), ('World', '/tag/world/'), ('Politics', '/tag/politics/'), ('Business', '/tag/business/'), ('Technology', '/tag/technology/'), ('Sports', '/tag/sports/'), ('Opinion', '/tag/opinion/'), ('Lifestyle', '/tag/lifestyle/')])
        e.menu('secondary_navigation', [('Sign up', '#/portal/signup'), ('About', '/about/'), ('RSS', '/rss/')])
    else:
        e.menu('navigation', [('U.S', '/tag/u-s/'), ('World', '/tag/world/'), ('Politics', '/tag/politics/'), ('Business', '/tag/business/'),
                              ('Technology', '/tag/technology/'), ('Sports', '/tag/sports/'), ('Opinion', '/tag/opinion/'), ('Video', '/tag/video/'),
                              ('Lifestyle', '/tag/lifestyle/'), ('Health', '/tag/health/')])
        e.menu('secondary_navigation', [
            ('#News', '#'), ('U.S', '/tag/u-s/'), ('World', '/tag/world/'), ('Politics', '/tag/politics/'), ('Education', '/tag/education/'), ('Sports', '/tag/sports/'), ('Business', '/tag/business/'), ('Tech', '/tag/technology/'),
            ('#Arts', '#'), ('Arts', '/tag/arts/'), ('Sunday Essay', '/tag/sunday-essay/'), ('Graphics', '/tag/graphics/'), ('Wirecutter', '/tag/wirecutter/'),
            ('#Lifestyle', '#'), ('Health', '/tag/health/'), ('Lifestyle', '/tag/lifestyle/'), ('Weather', '/tag/weather/'),
            ('#Opinion', '#'), ('Columns', '/tag/opinion/'), ('Guest Essays', '/tag/opinion/'), ('Video', '/tag/video/'),
            ('#More', '#'), ('About', '/about/'), ('Newsletter', '#/portal/signup'), ('Account', '#/portal/account'), ('RSS', '/rss/')])
        e.tier('Plus', 500, 5000, 'Every story, every newsletter.')
        e.tier('Pro', 1000, 10000, 'Plus the archive, events and audio.')

    e.write()


# ================================================================ Outrider
def outrider():
    e = Export('outrider')
    lead = e.user('Ada Lovelace', 'Twenty years of brand and product work. Still sketches everything on paper first.', 'Founder & creative director', face('women', 65))
    team = [lead, e.user('Marco Reyes', 'Leads identity and art direction.', 'Design director', face('men', 46)),
            e.user('Ines Okafor', 'Turns messy briefs into clear strategy.', 'Strategy lead', face('women', 33)),
            e.user('Tomas Berg', 'Builds the sites and products we design.', 'Engineering', face('men', 52))]

    def case(title, line, client, disciplines, days):
        blocks = [('p', line), ('h2', 'The brief'), ('p', f'{client} came to us with a problem they could describe in a paragraph but not a sentence. We spent the first week interviewing customers, staff and the board, then rewrote the problem in words everyone could repeat.'),
                  ('h2', 'What we made'), ('p', 'A system rather than a set of screens: type, colour, motion and components that the in-house team now extends on their own.'),
                  ('quote', 'The work was never the logo. It was the decisions behind it.'),
                  ('h2', 'Results'), ('ul', ['Launched in eleven weeks', 'Design system adopted by three product teams', 'Inbound enquiries up in the first quarter']),
                  ('p', 'We still work with the team on new launches every quarter.')]
        e.post(title, blocks, ['#work', client] + disciplines, [rng.choice(team), lead], line, pic(title), None, days < 30, 'public', days)

    rng = random.Random(3)
    case('Northwind Rebrand', 'A logistics giant that finally looks as fast as it moves. +38% inbound leads.', 'Northwind', ['Branding', 'Web'], 2)
    case('Lumen Health App', 'A calmer way to book care. Rated 4.8 by 60,000 patients in the first quarter.', 'Lumen', ['Product', 'Mobile'], 9)
    case('Halcyon Coffee', 'Packaging and a storefront for a roaster with nine cafés and a cult following.', 'Halcyon', ['Packaging', 'E-commerce'], 16)
    case('Orbital Annual Report', 'Thirty pages of numbers turned into a story people shared.', 'Orbital', ['Editorial', 'Motion'], 23)
    case('Fieldnote Platform', 'A design system and marketing site for a research tool used by 2,000 teams.', 'Fieldnote', ['Design system', 'Web'], 30)
    case('Parcel & Co Launch', 'Naming, identity and launch film for a courier start-up.', 'Parcel & Co', ['Naming', 'Film'], 37)
    for i, (t, x) in enumerate([('Brand strategy', 'Positioning, naming and the story your team can tell without notes.'),
                                ('Visual identity', 'Logos, type, colour and motion built as one system.'),
                                ('Digital product', 'Research, UX and interface design for apps and platforms.'),
                                ('Websites', 'Fast, accessible marketing sites, built and launched in-house.'),
                                ('Motion & film', 'Launch films, product demos and animated identities.')]):
        e.post(t, [('p', x), ('h2', 'How we work'), ('p', 'Every engagement starts with a two-week discovery sprint and ends with a hand-over your team can run with.'),
                   ('ul', ['Fixed scope and price, agreed up front', 'A senior designer on every call', 'Weekly show-and-tell'])],
               ['#service'], [lead], x, pic('service ' + t), days_ago=60 + i)
    for t, x, co in [('Maya Chen', 'They understood our business faster than our own board did, and the identity has carried us through two funding rounds.', 'Northwind'),
                     ('David Osei', 'Senior people, every meeting. No hand-offs to juniors, no surprises on the invoice, and the app shipped a week early.', 'Lumen Health'),
                     ('Sofia Lindqvist', 'The best agency relationship I have had in fifteen years of buying design.', 'Halcyon Coffee')]:
        e.post(t, [('p', x)], ['#testimonial', co], [lead], x, face('women' if t != 'David Osei' else 'men', 12 + len(t)), days_ago=70)
    for i, (t, x, tag) in enumerate([('Why we stopped doing pitches', 'Free work makes worse work. Here is what we do instead.', 'Studio'),
                                     ('The brief is the product', 'Most projects fail in the first meeting. A short guide to writing a brief that works.', 'Process'),
                                     ('What a design system is actually for', 'Not consistency. Speed, and the freedom to spend attention elsewhere.', 'Design'),
                                     ('Notes from a week without Figma', 'We went back to paper for five days. The work got better.', 'Studio')]):
        e.post(t, [('p', x), ('p', 'We have run the studio this way for twelve years and have the scars to prove it.'), ('h2', 'What we learned'),
                   ('p', 'Constraints are a gift, clients are partners, and the best idea in the room rarely arrives first.'),
                   ('quote', 'Good work is mostly the decisions you did not make.')],
               [tag], [team[i % 4]], x, pic('journal ' + t), days_ago=3 + i * 9)
    e.page('Work', [('p', 'A selection of projects from the last few years.')], 'work', 'Selected projects', [lead])
    e.page('Journal', [], 'journal', 'Notes from the studio', [lead])
    e.write()


# ================================================================ Monogram
def monogram():
    e = Export('monogram')
    me = e.user('Daniel Reyes', 'Product designer in Lisbon. I design calm, useful products and the brands around them.', 'Lisbon', face('men', 22))
    for i, (t, x) in enumerate([('Product design', 'From research to polished interface: apps and platforms people enjoy using.'),
                                ('Brand identity', 'Logos, type and colour systems with a reason behind every choice.'),
                                ('Design systems', 'Components and guidelines your team can actually maintain.'),
                                ('Web design', 'Fast, accessible websites built in Webflow, Framer or code.'),
                                ('Prototyping', 'Clickable prototypes to test ideas before they cost money.'),
                                ('Workshops', 'Half-day sessions to align a team on what to build next.')]):
        e.post(t, [('p', x), ('p', 'Projects start with a short call and a written proposal with a fixed price.')], ['#service'], [me], x, None, days_ago=90 + i)
    for i, (t, x, tag) in enumerate([('Atlas Finance Dashboard', 'Product design for a budgeting app used by 40,000 households.', 'App design'),
                                     ('Kinfolk Journal', 'Editorial website and CMS theme for an independent magazine.', 'Web design'),
                                     ('Moss & Stone', 'Identity for an architecture practice in Porto.', 'Branding'),
                                     ('Pulse Running Club', 'Mobile app and community platform for a running club with 12 chapters.', 'App design'),
                                     ('Tidewater Festival', 'Posters, wayfinding and website for a coastal music festival.', 'Branding'),
                                     ('Quiet Hours', 'Landing page and brand for a focus-timer app.', 'Web design')]):
        e.post(t, [('p', x), ('h2', 'The challenge'), ('p', 'The client needed something that felt considered without feeling expensive, and they needed it in six weeks.'),
                   ('h2', 'The approach'), ('p', 'We started with three directions on paper, chose one in a single meeting, and spent the rest of the time on details.'),
                   ('ul', ['Research and interviews', 'Identity and interface design', 'Hand-off with documented components']),
                   ('quote', 'The best design decisions are the ones you never have to explain.')],
               ['#project', tag], [me], x, pic(t), days_ago=5 + i * 12)
    for t, x in [('Lead product designer', 'Fieldnote · 2023 — now'), ('Senior designer', 'Studio Halden · 2020 — 2023'), ('Designer', 'Brightwave Agency · 2017 — 2020')]:
        e.post(t, [('p', 'Led design for the core product, hired and mentored a team of four, and set up the design system the company still runs on.')], ['#experience'], [me], x, days_ago=120)
    for t, x in [('MA Interaction Design', 'Royal College of Art · 2015 — 2017'), ('BA Graphic Design', 'University of the Arts · 2012 — 2015')]:
        e.post(t, [('p', 'Thesis on how interfaces teach people to trust them. Graduated with distinction.')], ['#education'], [me], x, days_ago=130)
    for t, x, co, img in [('Maya Chen', 'Daniel understood our product faster than anyone we have hired. The redesign paid for itself in a quarter.', 'Atlas Finance', face('women', 50)),
                          ('Ruben Costa', 'Calm, quick and opinionated in the best way. Our identity finally looks like us.', 'Moss & Stone', face('men', 41)),
                          ('Lena Fischer', 'The design system he set up is still the thing new hires thank us for.', 'Fieldnote', face('women', 29))]:
        e.post(t, [('p', x)], ['#testimonial', co], [me], x, img, days_ago=140)
    for i, (t, x, tag) in enumerate([('How I price design work', 'Fixed prices, written proposals and the one question I always ask first.', 'Freelancing'),
                                     ('Designing for calm', 'Notes on building products that do not shout for attention.', 'Design'),
                                     ('My tools in 2026', 'What I use every day, and what I have stopped using.', 'Tools')]):
        e.post(t, [('p', x), ('p', 'This is a short note from my own practice, not a rule for anyone else’s.'), ('h2', 'The short version'),
                   ('p', 'Be clear about scope, be generous with your time early, and write everything down.')], [tag], [me], x, pic('note ' + t), days_ago=1 + i * 10)
    e.page('Journal', [], 'journal', 'The notebook', [me])
    e.write()


# ================================================================ Plinth
def plinth():
    e = Export('plinth')
    ed = e.user('Hanna Berg', 'Editor of the journal and our head of product.', 'Portland', face('women', 57))
    for i, (t, x, tag) in enumerate([('A small living room that feels twice its size', 'Three moves that make a 12 m² room feel generous.', 'Spaces'),
                                     ('Inside the workshop where our chairs are made', 'Four people, one barn, and a lot of ash.', 'Makers'),
                                     ('How to choose a dining table that lasts', 'Wood, finish, legs and the question of extending.', 'Guides'),
                                     ('Lighting a room in three layers', 'Ambient, task and accent, explained with one lamp each.', 'Guides'),
                                     ('The quiet return of solid wood', 'Why veneer is losing ground, and what that costs.', 'Makers'),
                                     ('A reading corner in an afternoon', 'One chair, one lamp, one shelf.', 'Spaces')]):
        e.post(t, [('p', x), ('p', 'We visited homes, workshops and showrooms to find out what makes a room work.'), ('h2', 'What we learned'),
                   ('ul', ['Start with how you use the room', 'Buy fewer, better pieces', 'Leave space for the things you love']),
                   ('quote', 'A room is finished when nothing in it is waiting to be replaced.')], [tag], [ed], x, pic('home ' + t), featured=i < 2, days_ago=2 + i * 8)
    e.write()
    rows = []
    header = ['Handle', 'Title', 'Body (HTML)', 'Vendor', 'Type', 'Tags', 'Published', 'Option1 Name', 'Option1 Value',
              'Variant Price', 'Variant Compare At Price', 'Variant Inventory Qty', 'Variant Requires Shipping', 'Image Src', 'Status']
    products = [('turn-chair', 'Turn Chair Vivid', 'Chairs', 299, 0, ['Red', 'Black', 'Oak'], 'chair'),
                ('cross-chair-heritage', 'Cross Chair Heritage', 'Chairs', 188, 249, ['Oak', 'Walnut'], 'chair'),
                ('bay-sofa', 'Bay Sofa Basic', 'Sofas', 1289, 0, ['Sage', 'Sand'], 'sofa'),
                ('loop-armchair', 'Loop Lounge Armchair', 'Lounge Chairs', 649, 799, ['Tan', 'Olive'], 'armchair'),
                ('lune-lamp', 'Lune Table Lamp', 'Lighting', 149, 0, [], 'lamp'),
                ('arc-floor-lamp', 'Arc Floor Lamp', 'Lighting', 329, 389, ['Black', 'Brass'], 'lamp'),
                ('picnic-table', 'Picnic Table Oak', 'Tables', 890, 0, [], 'table'),
                ('writer-desk', 'Writer Desk', 'Desks', 549, 0, ['Oak', 'Ash'], 'desk'),
                ('axis-storage', 'Axis Storage System', 'Storage', 998, 1198, [], 'storage'),
                ('low-sideboard', 'Low Sideboard', 'Storage', 749, 0, ['Walnut', 'White'], 'sideboard'),
                ('pip-stool', 'Pip Bar Stool', 'Stools', 159, 199, ['Natural', 'Black'], 'stool'),
                ('long-bench', 'Long Bench', 'Benches', 279, 0, [], 'bench'),
                ('rib-vase', 'Rib Vase', 'Accessories', 49, 65, ['Ivory', 'Terracotta'], 'vase'),
                ('cloud-sofa', 'Cloud Three-Seater', 'Sofas', 1890, 0, ['Sand', 'Navy'], 'sofa'),
                ('fold-chair', 'Fold Chair', 'Chairs', 129, 0, [], 'chair'),
                ('cove-pendant', 'Cove Pendant', 'Lighting', 199, 0, [], 'light')]
    for i, (h, t, ty, price, cmp, colors, shape) in enumerate(products):
        desc = f'<p>{t} is built in a small workshop from solid timber and finished by hand. It is made to be used every day and to look better for it.</p><h3>Care</h3><p>Wipe with a dry cloth. Re-oil once a year.</p>'
        for j, c in enumerate(colors or ['Default Title']):
            first = j == 0
            rows.append([h, t if first else '', desc if first else '', 'Hearth & Grain' if first else '', ty if first else '',
                         f'shape:{shape}, {ty.lower()}' + (', featured' if i % 4 == 0 else '') if first else '', 'TRUE' if first else '',
                         ('Color' if colors else 'Title') if first else '', c, f'{price:.2f}', f'{cmp:.2f}' if cmp else '', 12, 'TRUE', '', 'active' if first else ''])
    path = os.path.join(ROOT, 'plinth', 'demo', 'plinth-products.csv')
    with open(path, 'w', newline='') as f:
        w = csv.writer(f)
        w.writerow(header)
        w.writerows(rows)
    print(f'{path}: {len(products)} products')


# ================================================================ Wayletter
def wayletter():
    e = Export('wayletter')
    me = e.user('Jamie Larson', 'Writes the weekly letter on technology, work and business.', 'Copenhagen', face('men', 36))
    lines = ['Most teams already know what to do; the hard part is deciding what to stop doing.',
             'The numbers are less dramatic than the headlines, and more useful.',
             'Here is what changed, what did not, and what I would do differently next time.',
             'If you only read one section, read the last one.']
    def issue(title, tags, excerpt, days, featured=False, visibility='public', wide=False):
        blocks = [('p', excerpt + ' ' + lines[days % 4]), ('p', lines[(days + 1) % 4] + ' ' + lines[(days + 2) % 4]),
                  ('h2', 'What I learned'), ('ul', ['Start smaller than feels comfortable', 'Write the decision down', 'Measure one thing, honestly']),
                  ('quote', 'Simplicity is not the absence of work. It is work, finished.'),
                  ('h2', 'Further reading'), ('p', 'Reply to this email with your own experience; I read every answer and share the best ones next week.')]
        return e.post(title, blocks, tags, [me], excerpt, pic(title, 1600, 800), None, featured, visibility, days)
    issue('AI product management: Research, requirements and scope', ['Technology'], 'Planning an AI feature starts with the problem, not the model.', 0)
    issue('Retailers going digital payment only don’t save money', ['Business'], 'Card-only stores promised savings. The ledgers tell a different story.', 2, True)
    issue('How to Ergonomically Optimize Your Workspace', ['Work'], 'Small changes to your desk add up to fewer aches by Friday.', 5)
    issue('When tech brands get illustration right', ['Design'], 'Why the best product illustrations feel human, and how to brief for them.', 9, True)
    issue('Tips for planning a successful business conference', ['Business'], 'A checklist from organising twelve events, including the ones that went wrong.', 11, True)
    issue('A gaming company’s office embraces an anti-tech atmosphere', ['Design', 'Work'], 'Plants, paper and a ban on screens in the kitchen.', 14, True)
    issue('The quiet comeback of the weekly email', ['Business', 'Technology'], 'Feeds come and go. The inbox keeps quietly winning.', 18, True)
    issue('Remote meetings that people actually want to attend', ['Work'], 'Shorter, written-first and with a clear owner.', 21)
    issue('Designing a calm notification strategy', ['Design', 'Technology'], 'Every ping is a withdrawal from your users’ attention.', 25, visibility='members')
    issue('What small companies get right about hiring', ['Business', 'Work'], 'They hire for the job in front of them, not the org chart they imagine.', 28)
    issue('The case for boring software', ['Technology'], 'Predictable tools free your attention for the interesting problems.', 32, True)
    issue('Pricing your first product without guessing', ['Business'], 'Three experiments that tell you more than any survey.', 36, visibility='paid')
    issue('How I take notes in meetings now', ['Work'], 'One page, three headings, and a rule about action items.', 40)
    issue('A short history of the open-plan office', ['Design', 'Work'], 'From factory floors to hot desks, and where it is heading.', 44)
    e.page('Style guide', [('p', 'This page shows how text looks in this theme: paragraphs, headings, quotes and lists.'),
                           ('h2', 'A second-level heading'), ('p', 'Body text is set at seventeen pixels with generous line height for comfortable reading.'),
                           ('h3', 'A third-level heading'), ('quote', 'A pull quote sits between paragraphs with an accent rule.'),
                           ('ul', ['A bulleted list item', 'Another item, a little longer to show wrapping across lines', 'A final item'])],
           'style-guide', 'How every element looks in Wayletter.', [me])
    e.menu('navigation', [('Home', '/'), ('About', '/about/'), ('Style Guide', '/style-guide/'), ('Author', '/author/jamie-larson/'),
                          ('Collection', '/tag/business/'), ('With wide image', '/when-tech-brands-get-illustration-right/'),
                          ('With full image', '/a-gaming-companys-office-embraces-an-anti-tech-atmosphere/'), ('Technology', '/tag/technology/'), ('Work', '/tag/work/')])
    e.menu('secondary_navigation', [('Terms', '/terms/'), ('Privacy', '/privacy/')])
    e.write()


if __name__ == '__main__':
    broadsheet()
    broadsheet('coverline')
    outrider()
    monogram()
    plinth()
    wayletter()
