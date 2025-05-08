from PySide6.QtWidgets import QCheckBox, QStyleOptionButton, QStyle
from PySide6.QtCore import QPropertyAnimation, QRectF, Property, QEasingCurve, Qt
from PySide6.QtGui import QPainter, QColor, QPen, QBrush

class AnimatedCheckBox(QCheckBox):
    def __init__(self, *args, colors=None, **kwargs):
        super().__init__(*args, **kwargs)
        self._progress = 1.0 if self.isChecked() else 0.0
        self._animation = QPropertyAnimation(self, b"progress", self)
        self._animation.setDuration(180)
        self._animation.setEasingCurve(QEasingCurve.OutCubic)
        self.stateChanged.connect(self.animate)
        self.setCursor(Qt.PointingHandCursor)
        self.setStyleSheet("QCheckBox::indicator { width: 0; height: 0; margin: 0; padding: 0; } QCheckBox { padding-left: 28px; }")
        self._colors = colors or {
            'bg': '#232a2e',
            'border': '#2ec27e',
            'checked_bg': '#8ff0a4',
            'checked_border': '#2ec27e',
            'checkmark': '#26a269',
            'hover': '#8ff0a4',
            'disabled_bg': '#273136',
            'disabled_border': '#2d353b',
            'checkmark_disabled': '#4b5a5e',
        }

    def set_colors(self, colors):
        self._colors = colors
        self.update()

    def animate(self, state):
        self._animation.stop()
        self._animation.setStartValue(self._progress)
        self._animation.setEndValue(1.0 if state == 2 else 0.0)
        self._animation.start()

    def getProgress(self):
        return self._progress

    def setProgress(self, value):
        self._progress = value
        self.update()

    progress = Property(float, getProgress, setProgress)

    def paintEvent(self, event):
        # Do NOT call super().paintEvent(event) to avoid native indicator
        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing)
        c = self._colors
        bg = QColor(c['bg'])
        border = QColor(c['border'])
        checked_bg = QColor(c['checked_bg'])
        checked_border = QColor(c['checked_border'])
        checkmark = QColor(c['checkmark'])
        hover = QColor(c['hover'])
        disabled_bg = QColor(c['disabled_bg'])
        disabled_border = QColor(c['disabled_border'])
        checkmark_disabled = QColor(c['checkmark_disabled'])
        if self.isEnabled():
            if self.underMouse():
                bg = hover
        else:
            bg = disabled_bg
            border = disabled_border
            checkmark = checkmark_disabled
        # Draw rounded box
        box_rect = QRectF(4, (self.height()-20)//2, 20, 20)
        painter.setPen(QPen(border, 2))
        painter.setBrush(QBrush(bg if self._progress < 0.5 else checked_bg))
        painter.drawRoundedRect(box_rect, 6, 6)
        # Draw checkmark with animation
        if self._progress > 0:
            painter.setPen(QPen(checkmark, 2.5))
            # Animate checkmark drawing
            p = self._progress
            # Checkmark points
            x0, y0 = box_rect.left()+5, box_rect.top()+10
            x1, y1 = box_rect.left()+10, box_rect.bottom()-5
            x2, y2 = box_rect.right()-5, box_rect.top()+5
            if p < 0.5:
                # Draw first segment
                x = x0 + (x1-x0) * (p/0.5)
                y = y0 + (y1-y0) * (p/0.5)
                painter.drawLine(x0, y0, x, y)
            else:
                # Draw full first segment
                painter.drawLine(x0, y0, x1, y1)
                # Draw second segment
                p2 = (p-0.5)/0.5
                x = x1 + (x2-x1) * p2
                y = y1 + (y2-y1) * p2
                painter.drawLine(x1, y1, x, y)
        # Draw the label (text)
        opt = QStyleOptionButton()
        self.initStyleOption(opt)
        # Move text right to leave space for custom box
        opt.rect = self.rect().adjusted(28, 0, 0, 0)
        self.style().drawControl(QStyle.CE_CheckBoxLabel, opt, painter, self)
        painter.end()

    def mousePressEvent(self, event):
        if self.isEnabled() and event.button() == Qt.LeftButton:
            self.toggle()
            self.clicked.emit()
        super().mousePressEvent(event) 