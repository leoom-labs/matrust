% UI Components Demo (Engine 9a3)
% Demonstrates every 9a1 widget, a uiaxes plot, and modal uiconfirm / uiprogressdlg sheets.

fig = uifigure('Name', 'UI Components Demo', 'Position', [100 100 920 680]);
grid = uigridlayout(fig, [5 3], ...
    'RowHeight', {'fit', 'fit', '1x', '1x', 'fit'}, ...
    'ColumnWidth', {220, '1x', '1x'}, ...
    'Padding', [12 12 12 12], ...
    'RowSpacing', 8, ...
    'ColumnSpacing', 8);

% Row 1: status label + push button + state button
statusLbl = uilabel(grid, 'Text', 'Ready', 'Tooltip', 'Current status');
statusLbl.Layout.Row = 1;
statusLbl.Layout.Column = 1;

pushBtn = uibutton(grid, 'push', 'Text', 'Click Me', 'Tooltip', 'Increment counter');
pushBtn.Layout.Row = 1;
pushBtn.Layout.Column = 2;
clickCount = 0;

stateBtn = uibutton(grid, 'state', 'Text', 'Toggle Mode', 'Value', false);
stateBtn.Layout.Row = 1;
stateBtn.Layout.Column = 3;

% Row 2: text edit, numeric edit, spinner
txtField = uieditfield(grid, 'text', 'Value', 'hello', 'Placeholder', 'Enter text');
txtField.Layout.Row = 2;
txtField.Layout.Column = 1;

numField = uieditfield(grid, 'numeric', 'Value', 10, 'Limits', [0 100]);
numField.Layout.Row = 2;
numField.Layout.Column = 2;
lastNumPrev = NaN;
lastNumVal = 10;

spin = uispinner(grid, 'Value', 5, 'Limits', [0 20], 'Step', 1);
spin.Layout.Row = 2;
spin.Layout.Column = 3;

% Row 3: panel (with checkbox, dropdown, switch, slider) + uiaxes
ctrlPanel = uipanel(grid, 'Title', 'Controls');
ctrlPanel.Layout.Row = 3;
ctrlPanel.Layout.Column = 1;
panelGrid = uigridlayout(ctrlPanel, [4 1], ...
    'RowHeight', {'fit', 'fit', 'fit', 'fit'}, ...
    'ColumnWidth', {'1x'}, ...
    'Padding', [8 8 8 8], ...
    'RowSpacing', 6);

chk = uicheckbox(panelGrid, 'Text', 'Show grid', 'Value', true);
chk.Layout.Row = 1;
chk.Layout.Column = 1;

dd = uidropdown(panelGrid, 'Items', {'Sine', 'Cosine', 'Square'}, 'Value', 'Sine');
dd.Layout.Row = 2;
dd.Layout.Column = 1;

sw = uiswitch(panelGrid, 'slider', 'Items', {'Off', 'On'}, 'Value', 'Off');
sw.Layout.Row = 3;
sw.Layout.Column = 1;

sld = uislider(panelGrid, 'Limits', [0 10], 'Value', 3, 'MajorTicks', [0 2 5 8 10]);
sld.Layout.Row = 4;
sld.Layout.Column = 1;

ax = uiaxes(grid);
ax.Layout.Row = 3;
ax.Layout.Column = [2 3];
t = linspace(0, 2*pi, 64);
plot(ax, t, sin(t));
title(ax, 'Signal Preview');
xlabel(ax, 't');
ylabel(ax, 'y');

% Row 4: radio button group + tab group (textarea, listbox) + editable table
bg = uibuttongroup(grid, 'Title', 'Options');
bg.Layout.Row = 4;
bg.Layout.Column = 1;
rb1 = uiradiobutton(bg, 'Text', 'Alpha');
rb2 = uiradiobutton(bg, 'Text', 'Beta');

tg = uitabgroup(grid);
tg.Layout.Row = 4;
tg.Layout.Column = 2;
tab1 = uitab(tg, 'Title', 'Notes');
ta = uitextarea(tab1, 'Value', {'Line 1', 'Line 2'}, 'Position', [8 8 180 120]);
tab2 = uitab(tg, 'Title', 'Items');
lb = uilistbox(tab2, 'Items', {'One', 'Two', 'Three'}, 'Value', {'One'}, ...
    'Multiselect', 'on', 'Position', [8 8 180 120]);

tbl = uitable(grid, ...
    'Data', [11 22; 33 44], ...
    'ColumnName', {'A', 'B'}, ...
    'ColumnEditable', [true true]);
tbl.Layout.Row = 4;
tbl.Layout.Column = 3;
lastCellEdit = [0 0 0];

% Row 5: dialog trigger buttons
dlgBtn = uibutton(grid, 'push', 'Text', 'Confirm & Progress');
dlgBtn.Layout.Row = 5;
dlgBtn.Layout.Column = [1 2];

alertBtn = uibutton(grid, 'push', 'Text', 'Show Alert');
alertBtn.Layout.Row = 5;
alertBtn.Layout.Column = 3;

% Wire callbacks
pushBtn.ButtonPushedFcn = @(src, ev) evalin('base', ...
    'clickCount = clickCount + 1; set(statusLbl, ''Text'', sprintf(''Clicked %d'', clickCount));');

numField.ValueChangedFcn = @(src, ev) evalin('base', ...
    sprintf('lastNumPrev = %g; lastNumVal = %g;', ev.PreviousValue, ev.Value));

tbl.CellEditCallback = @(src, ev) evalin('base', ...
    sprintf('lastCellEdit = [%g %g %g];', ev.Indices(1), ev.Indices(2), ev.NewData));

dlgBtn.ButtonPushedFcn = @(src, ev) runDemoDialogs(fig, statusLbl);
alertBtn.ButtonPushedFcn = @(src, ev) uialert(fig, 'Demo alert message.', 'Alert', 'Icon', 'info');

function runDemoDialogs(fig, statusLbl)
    choice = uiconfirm(fig, 'Run the progress demo?', 'Confirm Action', ...
        'Options', {'Run', 'Cancel'}, 'DefaultOption', 'Run');
    if strcmp(choice, 'Run')
        nSteps = 10;
        d = uiprogressdlg(fig, 'Title', 'Working', ...
            'Message', sprintf('Step 1 of %d', nSteps), ...
            'Value', 0, 'ShowPercentage', 'on', 'Cancelable', 'on');
        drawnow;
        for k = 1:nSteps
            if d.CancelRequested
                break;
            end
            d.Value = k / nSteps;
            d.Message = sprintf('Step %d of %d', k, nSteps);
            drawnow;
            pause(0.2);
        end
        wasCancelled = d.CancelRequested;
        close(d);
        if wasCancelled
            set(statusLbl, 'Text', 'Cancelled');
        else
            set(statusLbl, 'Text', 'Progress done');
        end
    else
        set(statusLbl, 'Text', 'Cancelled');
    end
end
