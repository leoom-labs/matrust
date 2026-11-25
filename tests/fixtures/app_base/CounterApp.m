classdef CounterApp < matlab.apps.AppBase

    % Properties that correspond to app components
    properties (Access = public)
        UIFigure        matlab.ui.Figure
        GridLayout      matlab.ui.container.GridLayout
        CountLabel      matlab.ui.control.Label
        StepEditField   matlab.ui.control.NumericEditField
        IncrementButton matlab.ui.control.Button
    end

    properties (Access = private)
        Count double = 0
        Step double = 1
    end

    % Callbacks that handle component events
    methods (Access = private)

        % Code that executes after component creation
        function startupFcn(app)
            app.Count = 10;
            app.Step = 2;
            app.StepEditField.Value = app.Step;
            app.CountLabel.Text = sprintf('Count: %d (step %d)', app.Count, app.Step);
        end

        % Button pushed function: IncrementButton
        function IncrementButtonPushed(app, event)
            app.Count = app.Count + app.Step;
            app.CountLabel.Text = sprintf('Count: %d (step %d)', app.Count, app.Step);
        end

        % Value changed function: StepEditField
        function StepEditFieldValueChanged(app, event)
            app.Step = event.Value;
            app.CountLabel.Text = sprintf('Count: %d (step %d)', app.Count, app.Step);
        end
    end

    % Component initialization
    methods (Access = private)

        % Create UIFigure and components
        function createComponents(app)
            app.UIFigure = uifigure('Visible', 'off');
            app.UIFigure.Position = [100 100 320 200];
            app.UIFigure.Name = 'Counter App';

            app.GridLayout = uigridlayout(app.UIFigure, [3 1]);

            app.CountLabel = uilabel(app.GridLayout);
            app.CountLabel.Text = 'Initial';

            app.StepEditField = uieditfield(app.GridLayout, 'numeric');
            app.StepEditField.ValueChangedFcn = createCallbackFcn(app, @StepEditFieldValueChanged, true);

            app.IncrementButton = uibutton(app.GridLayout, 'push');
            app.IncrementButton.Text = 'Increment';
            app.IncrementButton.ButtonPushedFcn = createCallbackFcn(app, @IncrementButtonPushed, true);

            app.UIFigure.Visible = 'on';
        end
    end

    % App creation and deletion
    methods (Access = public)

        function val = getCount(app)
            val = app.Count;
        end

        function val = getStep(app)
            val = app.Step;
        end

        % Construct app
        function app = CounterApp
            createComponents(app)
            registerApp(app, app.UIFigure)
            runStartupFcn(app, @startupFcn)

            if nargout == 0
                clear app
            end
        end

        % Code that executes before app deletion
        function delete(app)
            delete(app.UIFigure)
        end
    end
end
