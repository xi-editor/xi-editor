//
//  EditViewController.swift
//  XiEditor
//
//  Created by Brandon Titus on 11/2/16.
//  Copyright © 2016 Google. All rights reserved.
//

import Cocoa

class EditViewController: NSViewController {

    @IBOutlet var editView: EditView!
    @IBOutlet var shadowView: ShadowView!
    @IBOutlet var scrollView: NSScrollView!
    @IBOutlet var documentView: NSClipView!
    
    override func viewDidLoad() {
        super.viewDidLoad()
        
        self.shadowView.mouseUpHandler = editView.mouseUp(with:)
        self.shadowView.mouseDraggedHandler = editView.mouseDragged(with:)
        scrollView.contentView.documentCursor = NSCursor.iBeam();
        
        NotificationCenter.default.addObserver(self, selector: #selector(EditViewController.boundsDidChangeNotification(_:)), name: NSNotification.Name.NSViewBoundsDidChange, object: scrollView.contentView)
        NotificationCenter.default.addObserver(self, selector: #selector(EditViewController.frameDidChangeNotification(_:)), name: NSNotification.Name.NSViewFrameDidChange, object: scrollView)
    }
    
    func boundsDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }
    
    func frameDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }
    
    fileprivate func updateEditViewScroll() {
        editView?.updateScroll(scrollView.contentView.bounds)
        shadowView?.updateScroll(scrollView.contentView.bounds, scrollView.documentView!.bounds)
    }
}

// we set this in Document.swift when we load a new window or tab.
//TODO: will have to think about whether this will work with splits
extension EditViewController: NSWindowDelegate {
    func windowDidBecomeKey(_ notification: Notification) {
        editView.updateIsFrontmost(true)
    }

    func windowDidResignKey(_ notification: Notification) {
        editView.updateIsFrontmost(false);
    }
}
